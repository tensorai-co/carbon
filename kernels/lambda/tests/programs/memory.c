// Memory, for lambda: MAP_SHARED anonymous memory (and /dev/zero) shared with a child, pages touched before and after
// the fork alike, while MAP_PRIVATE isn't; memfd_create's file read and written and mapped, its mappings and its
// contents one, a private mapping copied, seals; memfd_secret; SysV shared memory between processes, with shmctl's
// numbers, keys, IPC_EXCL, IPC_RMID and shmdt; msync and mincore; mlock bringing pages in; mseal refusing what it
// should; NUMA policy on one node; no protection keys or shadow stacks; process_madvise, process_mrelease, cachestat
// and swapon. Freestanding, for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;

#if defined(__aarch64__)
enum { SYS_fcntl = 25, SYS_ftruncate = 46, SYS_openat = 56, SYS_close = 57, SYS_read = 63, SYS_write = 64,
       SYS_pread64 = 67, SYS_pwrite64 = 68, SYS_readlinkat = 78, SYS_fstat = 80, SYS_exit_group = 94, SYS_getpid = 172,
       SYS_shmget = 194, SYS_shmctl = 195, SYS_shmat = 196, SYS_shmdt = 197, SYS_munmap = 215, SYS_mremap = 216,
       SYS_clone = 220, SYS_mmap = 222, SYS_swapon = 224, SYS_mprotect = 226, SYS_msync = 227, SYS_mlock = 228,
       SYS_munlock = 229, SYS_mlockall = 230, SYS_munlockall = 231, SYS_mincore = 232, SYS_madvise = 233,
       SYS_mbind = 235, SYS_get_mempolicy = 236, SYS_set_mempolicy = 237, SYS_migrate_pages = 238,
       SYS_move_pages = 239, SYS_wait4 = 260, SYS_memfd_create = 279, SYS_mlock2 = 284, SYS_pkey_mprotect = 288,
       SYS_pkey_alloc = 289, SYS_pkey_free = 290, SYS_pidfd_open = 434, SYS_process_madvise = 440,
       SYS_memfd_secret = 447, SYS_process_mrelease = 448, SYS_set_mempolicy_home_node = 450, SYS_cachestat = 451,
       SYS_map_shadow_stack = 453, SYS_mseal = 462 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_fstat = 5, SYS_mmap = 9, SYS_mprotect = 10, SYS_munmap = 11,
       SYS_pread64 = 17, SYS_pwrite64 = 18, SYS_mremap = 25, SYS_msync = 26, SYS_mincore = 27, SYS_madvise = 28,
       SYS_shmget = 29, SYS_shmat = 30, SYS_shmctl = 31, SYS_getpid = 39, SYS_clone = 56, SYS_wait4 = 61,
       SYS_shmdt = 67, SYS_fcntl = 72, SYS_ftruncate = 77, SYS_mlock = 149, SYS_munlock = 150, SYS_mlockall = 151,
       SYS_munlockall = 152, SYS_swapon = 167, SYS_exit_group = 231, SYS_mbind = 237, SYS_set_mempolicy = 238,
       SYS_get_mempolicy = 239, SYS_migrate_pages = 256, SYS_openat = 257, SYS_readlinkat = 267, SYS_move_pages = 279,
       SYS_memfd_create = 319, SYS_mlock2 = 325, SYS_pkey_mprotect = 329, SYS_pkey_alloc = 330, SYS_pkey_free = 331,
       SYS_pidfd_open = 434, SYS_process_madvise = 440, SYS_memfd_secret = 447, SYS_process_mrelease = 448,
       SYS_set_mempolicy_home_node = 450, SYS_cachestat = 451, SYS_map_shadow_stack = 453, SYS_mseal = 462 };
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

#define AT_FDCWD (-100)
#define O_RDWR 2
#define O_CREAT 0100
#define O_TRUNC 01000
#define PROT_READ 1
#define PROT_WRITE 2
#define MAP_SHARED 1
#define MAP_PRIVATE 2
#define MAP_FIXED 0x10
#define MAP_ANONYMOUS 0x20
#define PAGE 4096
#define EPERM 1
#define ENOENT 2
#define ENODEV 19
#define EFAULT 14
#define EEXIST 17
#define EINVAL 22
#define ENOSPC 28
#define ENOMEM 12
#define EOPNOTSUPP 95
#define SIGCHLD 17
#define F_ADD_SEALS 1033
#define F_GET_SEALS 1034
#define F_SEAL_SEAL 1
#define F_SEAL_SHRINK 2
#define F_SEAL_GROW 4
#define F_SEAL_WRITE 8
#define IPC_PRIVATE 0
#define IPC_CREAT 01000
#define IPC_EXCL 02000
#define IPC_RMID 0
#define IPC_STAT 2
#define IPC_INFO 3

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
static void reap(i64 pid) { sys(SYS_wait4, pid, 0, 0, 0, 0, 0); }
static volatile char *map(u64 size, int prot, int flags, int fd) { return (volatile char *)sys(SYS_mmap, 0, size, prot, flags, fd, 0); }

void cmain(i64 *sp) {
    (void)sp;
    char buf[64];
    u64 st[32];
    int pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);

    // MAP_SHARED anonymous memory is shared with a child; MAP_PRIVATE isn't.
    volatile char *shared = map(4 * PAGE, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_ANONYMOUS, -1);
    volatile char *private = map(PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1);
    shared[0] = 42;
    private[0] = 7;
    i64 child = fork();
    if (child == 0) { shared[1] = 99; shared[3 * PAGE] = 55; private[0] = 8; quit(shared[0] == 42 ? 0 : 1); }
    reap(child);
    check(shared[1] == 99 && private[0] == 7, "MAP_SHARED anonymous: a child's write is the parent's; MAP_PRIVATE's isn't");
    check(shared[3 * PAGE] == 55, "a shared page first touched by the child after the fork is the parent's too");
    int zero = sys(SYS_openat, AT_FDCWD, (i64)"/dev/zero", O_RDWR, 0, 0, 0);
    volatile char *z = map(PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, zero);
    child = fork();
    if (child == 0) { z[10] = 'z'; quit(0); }
    reap(child);
    check(z[10] == 'z', "/dev/zero mapped shared is shared too");

    // memfd_create: a file in memory, its mappings and its contents one.
    int mfd = sys(SYS_memfd_create, (i64)"test", 1 | 2, 0, 0, 0, 0);
    check(mfd >= 0 && sys(SYS_write, mfd, (i64)"hello", 5, 0, 0, 0) == 5 && sys(SYS_fstat, mfd, (i64)st, 0, 0, 0, 0) == 0 &&
          *(i64 *)((char *)st + ST_SIZE) == 5 && (*(u32 *)((char *)st + ST_MODE) & 0170000) == 0100000, "memfd_create: written, a regular file of 5 bytes");
    i64 n;
    char fdlink[24] = "/proc/self/fd/";
    fdlink[14] = (char)('0' + mfd / 10 % 10); fdlink[15] = (char)('0' + mfd % 10); fdlink[16] = 0;
    if (mfd < 10) { fdlink[14] = (char)('0' + mfd); fdlink[15] = 0; }
    n = sys(SYS_readlinkat, AT_FDCWD, (i64)fdlink, (i64)buf, sizeof buf, 0, 0);
    check(n == 21 && same(buf, "/memfd:test (deleted)", 21), "its name, as /proc/self/fd says it");
    check(sys(SYS_ftruncate, mfd, 2 * PAGE, 0, 0, 0, 0) == 0, "ftruncate to two pages");
    volatile char *m1 = map(2 * PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, mfd);
    volatile char *m2 = map(2 * PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, mfd);
    check(same((char *)m1, "hello", 5), "mapped, it holds what was written");
    m1[0] = 'J';
    check(sys(SYS_pread64, mfd, (i64)buf, 5, 0, 0, 0) == 5 && same(buf, "Jello", 5) && m2[0] == 'J', "written through one mapping: read sees it, and so does another mapping");
    sys(SYS_pwrite64, mfd, (i64)"W", 1, PAGE + 7, 0, 0);
    check(m1[PAGE + 7] == 'W', "written by pwrite: the mapping sees it");
    volatile char *mp = map(PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE, mfd);
    mp[1] = 'X';
    check(mp[0] == 'J' && sys(SYS_pread64, mfd, (i64)buf, 2, 0, 0, 0) == 2 && buf[1] == 'e', "MAP_PRIVATE of it: a copy, once written");
    child = fork();
    if (child == 0) { m1[2] = 'L'; quit(0); }
    reap(child);
    check(sys(SYS_pread64, mfd, (i64)buf, 3, 0, 0, 0) == 3 && buf[2] == 'L', "a child's write through its inherited mapping is in the file");
    check(sys(SYS_fcntl, mfd, F_GET_SEALS, 0, 0, 0, 0) == 0 && sys(SYS_fcntl, mfd, F_ADD_SEALS, F_SEAL_SHRINK, 0, 0, 0) == 0 &&
          sys(SYS_ftruncate, mfd, PAGE, 0, 0, 0, 0) == -EPERM && sys(SYS_ftruncate, mfd, 3 * PAGE, 0, 0, 0, 0) == 0, "F_SEAL_SHRINK: no smaller, bigger still");
    check(sys(SYS_fcntl, mfd, F_ADD_SEALS, F_SEAL_WRITE | F_SEAL_GROW, 0, 0, 0) == 0 && sys(SYS_write, mfd, (i64)"x", 1, 0, 0, 0) == -EPERM &&
          sys(SYS_mmap, 0, PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, mfd, 0) == -EPERM && sys(SYS_mmap, 0, PAGE, PROT_READ, MAP_SHARED, mfd, 0) > 0,
          "F_SEAL_WRITE: no writes, no writable shared mapping; reading still");
    check(sys(SYS_fcntl, mfd, F_ADD_SEALS, F_SEAL_SEAL, 0, 0, 0) == 0 && sys(SYS_fcntl, mfd, F_ADD_SEALS, F_SEAL_GROW, 0, 0, 0) == -EPERM &&
          sys(SYS_fcntl, mfd, F_GET_SEALS, 0, 0, 0, 0) == (F_SEAL_SEAL | F_SEAL_SHRINK | F_SEAL_GROW | F_SEAL_WRITE), "F_SEAL_SEAL: no more seals; F_GET_SEALS");
    int plain = sys(SYS_memfd_create, (i64)"plain", 0, 0, 0, 0, 0);
    check(sys(SYS_fcntl, plain, F_GET_SEALS, 0, 0, 0, 0) == F_SEAL_SEAL && sys(SYS_fcntl, plain, F_ADD_SEALS, F_SEAL_WRITE, 0, 0, 0) == -EPERM,
          "without MFD_ALLOW_SEALING: sealed against sealing");
    char longname[300];
    for (int i = 0; i < 299; i++) longname[i] = 'a';
    longname[299] = 0;
    check(sys(SYS_memfd_create, (i64)longname, 0, 0, 0, 0, 0) == -EINVAL && sys(SYS_memfd_create, (i64)"x", 0x100, 0, 0, 0, 0) == -EINVAL,
          "memfd_create: a name too long, a bad flag");
    int secret = sys(SYS_memfd_secret, 0, 0, 0, 0, 0, 0);
    check(secret >= 0 && sys(SYS_ftruncate, secret, PAGE, 0, 0, 0, 0) == 0 && sys(SYS_read, secret, (i64)buf, 1, 0, 0, 0) == -EINVAL &&
          sys(SYS_write, secret, (i64)"x", 1, 0, 0, 0) == -EINVAL && sys(SYS_mmap, 0, PAGE, PROT_READ, MAP_PRIVATE, secret, 0) == -EINVAL, "memfd_secret: no read, write or private mapping");
    volatile char *s = map(PAGE, PROT_READ | PROT_WRITE, MAP_SHARED, secret);
    s[0] = 's';
    check(s[0] == 's', "memfd_secret: mapped shared and used");

    // SysV shared memory between processes.
    int id = sys(SYS_shmget, IPC_PRIVATE, 10000, 0600, 0, 0, 0);
    volatile char *seg = (volatile char *)sys(SYS_shmat, id, 0, 0, 0, 0, 0);
    check(id >= 0 && (i64)seg > 0, "shmget and shmat");
    seg[0] = 'p';
    child = fork();
    if (child == 0) {
        volatile char *again = (volatile char *)sys(SYS_shmat, id, 0, 0, 0, 0, 0);
        again[1] = 'c';
        seg[2] = 'i';
        quit(again[0] == 'p' ? 0 : 1);
    }
    int status = -1;
    sys(SYS_wait4, child, (i64)&status, 0, 0, 0, 0);
    check(status == 0 && seg[1] == 'c' && seg[2] == 'i', "a child's writes, by a new attachment and the inherited one, are the parent's");
    u64 ds[14];
    check(sys(SYS_shmctl, id, IPC_STAT, (i64)ds, 0, 0, 0) == 0 && ds[6] == 10000 && ds[11] == 1 && (int)ds[10] == pid && (((u32 *)ds)[5] & 0777) == 0600,
          "IPC_STAT: its size, one attachment, its creator, its mode");
    int keyed = sys(SYS_shmget, 0x1234, PAGE, IPC_CREAT | 0644, 0, 0, 0);
    check(keyed >= 0 && keyed != id && sys(SYS_shmget, 0x1234, PAGE, IPC_CREAT | IPC_EXCL | 0644, 0, 0, 0) == -EEXIST &&
          sys(SYS_shmget, 0x1234, 0, 0, 0, 0, 0) == keyed && sys(SYS_shmget, 0x1234, 2 * PAGE, 0, 0, 0, 0) == -EINVAL && sys(SYS_shmget, 0x4321, PAGE, 0, 0, 0, 0) == -ENOENT,
          "by key: IPC_CREAT, IPC_EXCL, found again, too big, not there");
    check(sys(SYS_shmctl, keyed, IPC_RMID, 0, 0, 0, 0) == 0 && sys(SYS_shmget, 0x1234, 0, 0, 0, 0, 0) == -ENOENT && sys(SYS_shmat, keyed, 0, 0, 0, 0, 0) == -EINVAL,
          "IPC_RMID: gone, by key and by ID");
    check(sys(SYS_shmdt, (i64)seg, 0, 0, 0, 0, 0) == 0 && sys(SYS_shmdt, (i64)seg, 0, 0, 0, 0, 0) == -EINVAL && sys(SYS_shmctl, id, IPC_STAT, (i64)ds, 0, 0, 0) == 0 && ds[11] == 0,
          "shmdt, twice EINVAL; no attachments left");
    check(sys(SYS_shmctl, 0, IPC_INFO, (i64)ds, 0, 0, 0) >= 0 && ds[0] > 0 && sys(SYS_shmctl, id, 99, 0, 0, 0, 0) == -EINVAL, "IPC_INFO; a bad command");
    sys(SYS_shmctl, id, IPC_RMID, 0, 0, 0, 0);

    // msync, mincore, mlock.
    volatile char *four = map(4 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1);
    check(sys(SYS_msync, (i64)four, PAGE, 4, 0, 0, 0) == 0 && sys(SYS_msync, (i64)four + 1, PAGE, 4, 0, 0, 0) == -EINVAL &&
          sys(SYS_msync, (i64)four, PAGE, 5, 0, 0, 0) == -EINVAL && sys(SYS_msync, 0x10000, PAGE, 4, 0, 0, 0) == -ENOMEM, "msync: fine, unaligned, ASYNC with SYNC, unmapped");
    four[0] = 1;
    four[2 * PAGE] = 1;
    unsigned char vec[4] = {9, 9, 9, 9};
    check(sys(SYS_mincore, (i64)four, 4 * PAGE, (i64)vec, 0, 0, 0) == 0 && vec[0] == 1 && vec[1] == 0 && vec[2] == 1 && vec[3] == 0, "mincore: the pages touched, and only them");
    check(sys(SYS_mincore, 0x10000, PAGE, (i64)vec, 0, 0, 0) == -ENOMEM && sys(SYS_mincore, (i64)four + 1, PAGE, (i64)vec, 0, 0, 0) == -EINVAL, "mincore: unmapped, unaligned");
    check(sys(SYS_mlock, (i64)four, 4 * PAGE, 0, 0, 0, 0) == 0 && sys(SYS_mincore, (i64)four, 4 * PAGE, (i64)vec, 0, 0, 0) == 0 && vec[1] == 1 && vec[3] == 1,
          "mlock brings the pages in");
    volatile char *lazy = map(2 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1);
    check(sys(SYS_mlock2, (i64)lazy, 2 * PAGE, 1, 0, 0, 0) == 0 && sys(SYS_mincore, (i64)lazy, 2 * PAGE, (i64)vec, 0, 0, 0) == 0 && vec[0] == 0, "mlock2 with MLOCK_ONFAULT doesn't");
    check(sys(SYS_munlock, (i64)four, 4 * PAGE, 0, 0, 0, 0) == 0 && sys(SYS_mlock, 0x10000, PAGE, 0, 0, 0, 0) == -ENOMEM && sys(SYS_mlockall, 1, 0, 0, 0, 0, 0) == 0 &&
          sys(SYS_mlockall, 0, 0, 0, 0, 0, 0) == -EINVAL && sys(SYS_munlockall, 0, 0, 0, 0, 0, 0) == 0, "munlock, mlock unmapped, mlockall, munlockall");

    // mseal.
    volatile char *sealed = map(2 * PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1);
    check(sys(SYS_mseal, (i64)sealed, 2 * PAGE, 0, 0, 0, 0) == 0, "mseal");
    check(sys(SYS_munmap, (i64)sealed, PAGE, 0, 0, 0, 0) == -EPERM && sys(SYS_mprotect, (i64)sealed, PAGE, PROT_READ | PROT_WRITE, 0, 0, 0) == -EPERM &&
          sys(SYS_mmap, (i64)sealed, PAGE, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0) == -EPERM &&
          sys(SYS_mremap, (i64)sealed, 2 * PAGE, 4 * PAGE, 1, 0, 0) == -EPERM && sys(SYS_madvise, (i64)sealed, PAGE, 4, 0, 0, 0) == -EPERM,
          "sealed: no munmap, mprotect, mmap over it, mremap or MADV_DONTNEED");
    check(sys(SYS_mseal, (i64)sealed + 1, PAGE, 0, 0, 0, 0) == -EINVAL && sys(SYS_mseal, (i64)sealed, PAGE, 1, 0, 0, 0) == -EINVAL &&
          sys(SYS_mseal, 0x10000, PAGE, 0, 0, 0, 0) == -ENOMEM, "mseal: unaligned, a flag, unmapped");

    // NUMA, with one node.
    int mode = -1;
    u64 nodes = 99, one = 1, two = 2;
    check(sys(SYS_get_mempolicy, (i64)&mode, (i64)&nodes, 64, 0, 0, 0) == 0 && mode == 0 && nodes == 0, "get_mempolicy: MPOL_DEFAULT");
    check(sys(SYS_set_mempolicy, 2, (i64)&one, 64, 0, 0, 0) == 0 && sys(SYS_get_mempolicy, (i64)&mode, (i64)&nodes, 64, 0, 0, 0) == 0 && mode == 2 && nodes == 1,
          "set_mempolicy MPOL_BIND to node 0, read back");
    check(sys(SYS_set_mempolicy, 2, (i64)&two, 64, 0, 0, 0) == -EINVAL && sys(SYS_set_mempolicy, 0, (i64)&one, 64, 0, 0, 0) == -EINVAL &&
          sys(SYS_set_mempolicy, 9, 0, 0, 0, 0, 0) == -EINVAL, "set_mempolicy: a node that isn't, nodes for MPOL_DEFAULT, no such mode");
    check(sys(SYS_get_mempolicy, (i64)&mode, (i64)&nodes, 64, 0, 4, 0) == 0 && nodes == 1 && sys(SYS_get_mempolicy, (i64)&mode, 0, 0, (i64)four, 3, 0) == 0 && mode == 0,
          "get_mempolicy: the nodes allowed; a page's node");
    check(sys(SYS_mbind, (i64)four, 4 * PAGE, 2, (i64)&one, 64, 0) == 0 && sys(SYS_mbind, 0x10000, PAGE, 2, (i64)&one, 64, 0) == -EFAULT, "mbind: a mapped range, an unmapped one");
    void *pages[2] = {(void *)four, (void *)0x10000};
    int status2[2] = {9, 9}, to[2] = {1, 0};
    check(sys(SYS_move_pages, 0, 2, (i64)pages, 0, (i64)status2, 0) == 0 && status2[0] == 0 && status2[1] == -EFAULT &&
          sys(SYS_move_pages, 0, 1, (i64)pages, (i64)to, (i64)status2, 2) == 0 && status2[0] == -ENODEV, "move_pages: node 0, unmapped, and no node 1 to move to");
    check(sys(SYS_set_mempolicy_home_node, (i64)four, PAGE, 0, 0, 0, 0) == 0 && sys(SYS_set_mempolicy_home_node, (i64)four, PAGE, 1, 0, 0, 0) == -EINVAL &&
          sys(SYS_migrate_pages, 0, 64, (i64)&one, (i64)&one, 0, 0) == 0, "set_mempolicy_home_node and migrate_pages");
    sys(SYS_set_mempolicy, 0, 0, 0, 0, 0, 0);

    // No protection keys or shadow stacks; the rest.
    check(sys(SYS_pkey_alloc, 0, 0, 0, 0, 0, 0) == -ENOSPC && sys(SYS_pkey_free, 1, 0, 0, 0, 0, 0) == -EINVAL &&
          sys(SYS_pkey_mprotect, (i64)lazy, PAGE, PROT_READ, -1, 0, 0) == 0 && sys(SYS_pkey_mprotect, (i64)lazy, PAGE, PROT_READ, 1, 0, 0) == -EINVAL,
          "pkey_alloc ENOSPC, pkey_free EINVAL; pkey_mprotect with no key is mprotect");
    check(sys(SYS_map_shadow_stack, 0, PAGE, 0, 0, 0, 0) == -EOPNOTSUPP && sys(SYS_swapon, (i64)"/swap", 0, 0, 0, 0, 0) == -EPERM, "map_shadow_stack EOPNOTSUPP; swapon EPERM");
    int self = sys(SYS_pidfd_open, pid, 0, 0, 0, 0, 0);
    struct { void *base; u64 len; } iov = {(void *)four, 4 * PAGE};
    check(sys(SYS_process_madvise, self, (i64)&iov, 1, 20, 0, 0) == 4 * PAGE && sys(SYS_process_madvise, self, (i64)&iov, 1, 4, 0, 0) == -EINVAL &&
          sys(SYS_process_mrelease, self, 0, 0, 0, 0, 0) == -EINVAL, "process_madvise MADV_COLD; not MADV_DONTNEED; process_mrelease of one that isn't ending");
    u64 range[2] = {0, 0}, cs[5] = {9, 9, 9, 9, 9};
    int cfile = sys(SYS_openat, AT_FDCWD, (i64)"/tmp/cached", O_RDWR | O_CREAT | O_TRUNC, 0644, 0, 0);
    sys(SYS_pwrite64, cfile, (i64)"x", 1, 9999, 0, 0);
    check(sys(SYS_cachestat, cfile, (i64)range, (i64)cs, 0, 0, 0) == 0 && cs[0] == 3 && cs[1] == 0 && sys(SYS_cachestat, mfd, (i64)range, (i64)cs, 0, 0, 0) == 0 && cs[0] == 2,
          "cachestat: a 10,000-byte file's 3 pages; a memfd's 2 written");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
