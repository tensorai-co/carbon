// Security and the machine's administration, for lambda. Capabilities: capget's full set for root and its version
// handshake, ESRCH; capset dropping CAP_SETUID so setuid fails, and putting it back; SECBIT_KEEP_CAPS through leaving
// root, then CAP_SETUID alone getting root back; ambient raise, is-set, lower and clear; the bounding set; /proc's
// Cap lines and cap_last_cap. prctl: a name cut to 15 bytes, keep-caps and securebits, no_new_privs. seccomp: actions
// available, bad programs and flags refused; a filter's errno for one call, inherited by a child; a second filter's
// SIGSYS with its siginfo, the most restrictive answer winning; a call killing the process with SIGSYS; strict mode's
// SIGKILL; EACCES without privilege or no_new_privs. The machine: host and domain names in uname and /proc/sys;
// reboot's magic; the kernel log; no quotas; module loading, kexec, accounting, BPF and perf refused; LSM's one
// module; Landlock off; and, without root, EPERM for the rest. On x86-64, iopl and ioperm. Freestanding, for ARM64
// and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;

#if defined(__aarch64__)
enum { SYS_quotactl = 60, SYS_openat = 56, SYS_close = 57, SYS_vhangup = 58, SYS_read = 63, SYS_write = 64,
       SYS_acct = 89, SYS_capget = 90, SYS_capset = 91, SYS_exit = 93, SYS_exit_group = 94, SYS_kexec_load = 104,
       SYS_init_module = 105, SYS_syslog = 116, SYS_rt_sigaction = 134, SYS_reboot = 142, SYS_setuid = 146,
       SYS_setresuid = 147, SYS_uname = 160, SYS_sethostname = 161, SYS_setdomainname = 162, SYS_prctl = 167,
       SYS_getpid = 172, SYS_getppid = 173, SYS_getuid = 174, SYS_clone = 220, SYS_perf_event_open = 241,
       SYS_wait4 = 260, SYS_seccomp = 277, SYS_bpf = 280, SYS_landlock_create_ruleset = 444,
       SYS_lsm_get_self_attr = 459, SYS_lsm_set_self_attr = 460, SYS_lsm_list_modules = 461 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ARCH 0xc00000b7u
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_rt_sigaction = 13, SYS_getpid = 39, SYS_clone = 56,
       SYS_exit = 60, SYS_wait4 = 61, SYS_uname = 63, SYS_getuid = 102, SYS_setuid = 105, SYS_getppid = 110,
       SYS_setresuid = 117, SYS_capget = 125, SYS_capset = 126, SYS_vhangup = 153, SYS_prctl = 157, SYS_acct = 163,
       SYS_reboot = 169, SYS_sethostname = 170, SYS_setdomainname = 171, SYS_iopl = 172, SYS_ioperm = 173,
       SYS_init_module = 175, SYS_quotactl = 179, SYS_syslog = 103, SYS_exit_group = 231, SYS_kexec_load = 246,
       SYS_openat = 257, SYS_perf_event_open = 298, SYS_seccomp = 317, SYS_bpf = 321,
       SYS_landlock_create_ruleset = 444, SYS_lsm_get_self_attr = 459, SYS_lsm_set_self_attr = 460,
       SYS_lsm_list_modules = 461 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ARCH 0xc000003eu
#endif

#define AT_FDCWD (-100)
#define EPERM 1
#define ESRCH 3
#define E2BIG 7
#define EACCES 13
#define EINVAL 22
#define ENOSYS 38
#define EOPNOTSUPP 95
#define SIGKILL 9
#define SIGCHLD 17
#define SIGSYS 31
#define SA_SIGINFO 4
#define CAP_V3 0x20080522
#define ALL_LOW 0xffffffffu
#define ALL_HIGH 0x1ffu
#define CAP_SETUID 7
#define CAP_NET_BIND_SERVICE 10
#define PR_SET_KEEPCAPS 8
#define PR_GET_KEEPCAPS 7
#define PR_SET_NAME 15
#define PR_GET_NAME 16
#define PR_GET_SECCOMP 21
#define PR_CAPBSET_READ 23
#define PR_GET_SECUREBITS 27
#define PR_SET_NO_NEW_PRIVS 38
#define PR_GET_NO_NEW_PRIVS 39
#define PR_CAP_AMBIENT 47
#define SECCOMP_RET_KILL_PROCESS 0x80000000u
#define SECCOMP_RET_TRAP 0x00030000u
#define SECCOMP_RET_ERRNO 0x00050000u
#define SECCOMP_RET_USER_NOTIF 0x7fc00000u
#define SECCOMP_RET_ALLOW 0x7fff0000u

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
static int reap(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return status; }
static i64 slurp(const char *path, char *buf, u64 size) {
    int fd = sys(SYS_openat, AT_FDCWD, (i64)path, 0, 0, 0, 0);
    i64 n = sys(SYS_read, fd, (i64)buf, size, 0, 0, 0);
    sys(SYS_close, fd, 0, 0, 0, 0, 0);
    return n;
}
static i64 prctl(i64 option, i64 a, i64 b) { return sys(SYS_prctl, option, a, b, 0, 0, 0); }

struct cap_header { u32 version; int pid; };
struct cap_data { u32 effective, permitted, inheritable; };
static int capset2(u64 e, u64 p, u64 i) {
    struct cap_header h = {CAP_V3, 0};
    struct cap_data d[2] = {{(u32)e, (u32)p, (u32)i}, {(u32)(e >> 32), (u32)(p >> 32), (u32)(i >> 32)}};
    return sys(SYS_capset, (i64)&h, (i64)d, 0, 0, 0, 0);
}
static u64 caps_of(int which) {
    struct cap_header h = {CAP_V3, 0};
    struct cap_data d[2];
    sys(SYS_capget, (i64)&h, (i64)d, 0, 0, 0, 0);
    u32 lo = which == 0 ? d[0].effective : which == 1 ? d[0].permitted : d[0].inheritable;
    u32 hi = which == 0 ? d[1].effective : which == 1 ? d[1].permitted : d[1].inheritable;
    return (u64)hi << 32 | lo;
}
#define ALL ((u64)ALL_HIGH << 32 | ALL_LOW)

struct sock_filter { u16 code; unsigned char jt, jf; u32 k; };
struct sock_fprog { u16 len; struct sock_filter *filter; };
#define LD_NR {0x20, 0, 0, 0}
#define RET(v) {0x06, 0, 0, (v)}
#define JEQ(k, t, f) {0x15, (t), (f), (k)}
static int install(struct sock_filter *insns, u16 n) {
    struct sock_fprog prog = {n, insns};
    return sys(SYS_seccomp, 1, 0, (i64)&prog, 0, 0, 0);
}

// What the SIGSYS handler saw.
static volatile int got_signo, got_errno, got_code, got_syscall;
static volatile u32 got_arch;
static volatile u64 got_addr;
static void on_sigsys(int signo, int *info, void *context) {
    (void)signo; (void)context;
    got_signo = info[0];
    got_errno = info[1];
    got_code = info[2];
    got_addr = *(u64 *)(info + 4);
    got_syscall = info[6];
    got_arch = (u32)info[7];
}

void cmain(i64 *sp) {
    (void)sp;
    char buf[4096];
    int pid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);

    // Capabilities.
    struct cap_header h = {CAP_V3, 0};
    struct cap_data d[2];
    check(sys(SYS_capget, (i64)&h, (i64)d, 0, 0, 0, 0) == 0 && d[0].effective == ALL_LOW && d[1].effective == ALL_HIGH && d[0].permitted == ALL_LOW &&
          d[1].permitted == ALL_HIGH && d[0].inheritable == 0, "capget: root has every capability, effective and permitted, through CAP_CHECKPOINT_RESTORE");
    struct cap_header odd = {0x12345678, 0};
    check(sys(SYS_capget, (i64)&odd, (i64)d, 0, 0, 0, 0) == -EINVAL && odd.version == CAP_V3, "a version it doesn't know: EINVAL, with the one it prefers written back");
    odd.version = 0x12345678;
    struct cap_header other = {CAP_V3, 999999};
    check(sys(SYS_capget, (i64)&odd, 0, 0, 0, 0, 0) == 0 && odd.version == CAP_V3 && sys(SYS_capget, (i64)&other, (i64)d, 0, 0, 0, 0) == -ESRCH,
          "asking only the version: 0; another pid that isn't: ESRCH");
    i64 n = slurp("/proc/self/status", buf, sizeof buf);
    check(contains(buf, n, "CapEff:\t000001ffffffffff\n") && contains(buf, n, "CapBnd:\t000001ffffffffff\n") && contains(buf, n, "NoNewPrivs:\t0\n") &&
          contains(buf, n, "Seccomp:\t0\n"), "/proc/self/status: CapEff, CapBnd, NoNewPrivs, Seccomp");
    n = slurp("/proc/sys/kernel/cap_last_cap", buf, sizeof buf);
    check(n == 3 && same(buf, "40\n", 3) && prctl(PR_CAPBSET_READ, CAP_SETUID, 0) == 1 && prctl(PR_CAPBSET_READ, 41, 0) == -EINVAL,
          "cap_last_cap is 40; PR_CAPBSET_READ");
    i64 child = fork();
    if (child == 0) {
        int bad = 0;
        if (capset2(ALL & ~(1ul << CAP_SETUID), ALL, 0) != 0 || sys(SYS_setuid, 1000, 0, 0, 0, 0, 0) != -EPERM) bad |= 1;
        if (capset2(ALL, ALL, 0) != 0 || capset2(ALL, ALL, 1ul << 50) != -EPERM) bad |= 2;
        if (prctl(PR_SET_KEEPCAPS, 1, 0) != 0 || prctl(PR_GET_KEEPCAPS, 0, 0) != 1 || prctl(PR_GET_SECUREBITS, 0, 0) != 0x10) bad |= 4;
        if (sys(SYS_setresuid, 1000, 1000, 1000, 0, 0, 0) != 0 || caps_of(0) != 0 || caps_of(1) != ALL) bad |= 8;
        if (capset2(1ul << CAP_SETUID, 1ul << CAP_SETUID, 0) != 0 || sys(SYS_setuid, 0, 0, 0, 0, 0, 0) != 0 || sys(SYS_getuid, 0, 0, 0, 0, 0, 0) != 0 ||
            caps_of(0) != 1ul << CAP_SETUID)
            bad |= 16;
        quit(bad);
    }
    int status = reap(child);
    check(status == 0, "capset drops CAP_SETUID and setuid fails; KEEPCAPS keeps the permitted set through leaving root; CAP_SETUID alone gets root back");
    child = fork();
    if (child == 0) {
        int bad = 0;
        if (capset2(ALL, ALL, 1ul << CAP_NET_BIND_SERVICE) != 0) bad |= 1;
        if (sys(SYS_prctl, PR_CAP_AMBIENT, 2, CAP_NET_BIND_SERVICE, 0, 0, 0) != 0 || sys(SYS_prctl, PR_CAP_AMBIENT, 1, CAP_NET_BIND_SERVICE, 0, 0, 0) != 1) bad |= 2;
        if (sys(SYS_prctl, PR_CAP_AMBIENT, 2, 11, 0, 0, 0) != -EPERM) bad |= 4;
        if (sys(SYS_prctl, PR_CAP_AMBIENT, 4, 0, 0, 0, 0) != 0 || sys(SYS_prctl, PR_CAP_AMBIENT, 1, CAP_NET_BIND_SERVICE, 0, 0, 0) != 0) bad |= 8;
        quit(bad);
    }
    check(reap(child) == 0, "ambient: raised (in permitted and inheritable), is-set, refused when not inheritable, cleared");

    // prctl.
    char name[16] = {0};
    check(prctl(PR_SET_NAME, (i64)"watchful-name-too-long", 0) == 0 && prctl(PR_GET_NAME, (i64)name, 0) == 0 && same(name, "watchful-name-t", 16),
          "PR_SET_NAME cuts to 15 bytes; PR_GET_NAME");
    n = slurp("/proc/self/status", buf, sizeof buf);
    check(contains(buf, n, "Name:\twatchful-name-t\n"), "/proc/self/status has the name");

    // seccomp.
    u32 action = SECCOMP_RET_KILL_PROCESS, notif = SECCOMP_RET_USER_NOTIF;
    struct sock_filter jumps_out[] = {JEQ(1, 5, 0), RET(SECCOMP_RET_ALLOW)};
    struct sock_filter allow[] = {RET(SECCOMP_RET_ALLOW)};
    struct sock_fprog prog = {1, allow};
    check(sys(SYS_seccomp, 2, 0, (i64)&action, 0, 0, 0) == 0 && sys(SYS_seccomp, 2, 0, (i64)&notif, 0, 0, 0) == -EOPNOTSUPP &&
          install(jumps_out, 2) == -EINVAL && sys(SYS_seccomp, 1, 8, (i64)&prog, 0, 0, 0) == -EINVAL && sys(SYS_seccomp, 9, 0, 0, 0, 0, 0) == -EINVAL &&
          prctl(PR_GET_SECCOMP, 0, 0) == 0, "seccomp: actions available, a jump out of the program, NEW_LISTENER, an operation that isn't");
    child = fork();
    if (child == 0) {
        int bad = 0;
        struct sock_filter first[] = {{0x20, 0, 0, 4}, JEQ(ARCH, 1, 0), RET(SECCOMP_RET_KILL_PROCESS), LD_NR, JEQ(SYS_getppid, 0, 1),
                                      RET(SECCOMP_RET_ERRNO | EACCES), RET(SECCOMP_RET_ALLOW)};
        if (install(first, 7) != 0 || sys(SYS_getppid, 0, 0, 0, 0, 0, 0) != -EACCES || sys(SYS_getpid, 0, 0, 0, 0, 0, 0) <= 0) bad |= 1;
        if (prctl(PR_GET_SECCOMP, 0, 0) != 2) bad |= 2;
        char status_buf[2048];
        i64 sn = slurp("/proc/self/status", status_buf, sizeof status_buf);
        if (!contains(status_buf, sn, "Seccomp:\t2\n") || !contains(status_buf, sn, "Seccomp_filters:\t1\n")) bad |= 4;
        i64 grandchild = fork();
        if (grandchild == 0) quit(sys(SYS_getppid, 0, 0, 0, 0, 0, 0) == -EACCES ? 0 : 1);
        if (reap(grandchild) != 0) bad |= 8;
        struct { void *handler; u64 flags; void *restorer; u64 mask; } act = {(void *)on_sigsys, SA_SIGINFO, 0, 0};
        sys(SYS_rt_sigaction, SIGSYS, (i64)&act, 0, 8, 0, 0);
        struct sock_filter second[] = {LD_NR, JEQ(SYS_uname, 0, 1), RET(SECCOMP_RET_TRAP | 42), RET(SECCOMP_RET_ALLOW)};
        if (install(second, 4) != 0) bad |= 16;
        if (sys(SYS_uname, (i64)status_buf, 0, 0, 0, 0, 0) != -ENOSYS || got_signo != SIGSYS || got_errno != 42 || got_code != 1 ||
            got_syscall != SYS_uname || got_arch != ARCH || got_addr == 0)
            bad |= 32;
        if (sys(SYS_getppid, 0, 0, 0, 0, 0, 0) != -EACCES) bad |= 64;
        quit(bad);
    }
    status = reap(child);
    check(status == 0, "a filter's errno, kept by a child; a second filter's SIGSYS with si_errno, SYS_SECCOMP, the call, its arch and address; the most restrictive wins");
    child = fork();
    if (child == 0) {
        struct sock_filter kill[] = {LD_NR, JEQ(SYS_getuid, 0, 1), RET(SECCOMP_RET_KILL_PROCESS), RET(SECCOMP_RET_ALLOW)};
        install(kill, 4);
        sys(SYS_getuid, 0, 0, 0, 0, 0, 0);
        quit(0);
    }
    status = reap(child);
    check((status & 0x7f) == SIGSYS, "SECCOMP_RET_KILL_PROCESS: killed by SIGSYS");
    child = fork();
    if (child == 0) {
        if (sys(SYS_seccomp, 0, 0, 0, 0, 0, 0) != 0) sys(SYS_exit, 1, 0, 0, 0, 0, 0);
        sys(SYS_write, 1, (i64)"", 0, 0, 0, 0);
        sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
        sys(SYS_exit, 2, 0, 0, 0, 0, 0);
    }
    status = reap(child);
    check((status & 0x7f) == SIGKILL, "strict mode: write is allowed, getpid gets SIGKILL");
    child = fork();
    if (child == 0) {
        int bad = 0;
        sys(SYS_setresuid, 1000, 1000, 1000, 0, 0, 0);
        if (install(allow, 1) != -EACCES) bad |= 1;
        if (prctl(PR_GET_NO_NEW_PRIVS, 0, 0) != 0 || prctl(PR_SET_NO_NEW_PRIVS, 2, 0) != -EINVAL || prctl(PR_SET_NO_NEW_PRIVS, 1, 0) != 0 ||
            prctl(PR_GET_NO_NEW_PRIVS, 0, 0) != 1)
            bad |= 2;
        if (install(allow, 1) != 0) bad |= 4;
        // And the machine's administration, without root.
        if (sys(SYS_sethostname, (i64)"x", 1, 0, 0, 0, 0) != -EPERM || sys(SYS_reboot, 0xfee1dead, 672274793, 0, 0, 0, 0) != -EPERM ||
            sys(SYS_syslog, 5, 0, 0, 0, 0, 0) != -EPERM || sys(SYS_syslog, 10, 0, 0, 0, 0, 0) != 131072 || sys(SYS_vhangup, 0, 0, 0, 0, 0, 0) != -EPERM)
            bad |= 8;
        quit(bad);
    }
    check(reap(child) == 0, "without privilege: EACCES until no_new_privs; then a filter. And EPERM for hostname, reboot, clearing the log, vhangup");

    // The machine.
    struct { char f[6][65]; } uts;
    check(sys(SYS_sethostname, (i64)"lambda-test", 11, 0, 0, 0, 0) == 0 && sys(SYS_setdomainname, (i64)"example", 7, 0, 0, 0, 0) == 0 &&
          sys(SYS_sethostname, (i64)buf, 65, 0, 0, 0, 0) == -EINVAL && sys(SYS_uname, (i64)&uts, 0, 0, 0, 0, 0) == 0 &&
          same(uts.f[1], "lambda-test", 12) && same(uts.f[5], "example", 8), "sethostname and setdomainname, as uname has them; too long, EINVAL");
    n = slurp("/proc/sys/kernel/hostname", buf, sizeof buf);
    check(n == 12 && same(buf, "lambda-test\n", 12), "/proc/sys/kernel/hostname");
    check(sys(SYS_reboot, 0xfee1dead, 1, 0, 0, 0, 0) == -EINVAL && sys(SYS_reboot, 0xfee1dead, 672274793, 0, 0, 0, 0) == 0 &&
          sys(SYS_reboot, 0xfee1dead, 672274793, 0xd000fce2, 0, 0, 0) == -EINVAL, "reboot: the wrong magic, Ctrl-Alt-Del off, suspending refused");
    n = sys(SYS_syslog, 3, (i64)buf, sizeof buf, 0, 0, 0);
    check(sys(SYS_syslog, 10, 0, 0, 0, 0, 0) == 131072 && n > 0 && contains(buf, n, "Linux version") && sys(SYS_syslog, 9, 0, 0, 0, 0, 0) == n &&
          sys(SYS_syslog, 2, (i64)buf, sizeof buf, 0, 0, 0) == n && sys(SYS_syslog, 9, 0, 0, 0, 0, 0) == 0 && sys(SYS_syslog, 8, 0, 9, 0, 0, 0) == -EINVAL,
          "syslog: the buffer's size, the log as dmesg reads it, unread, read, a console level that isn't");
    check(sys(SYS_quotactl, 0x80000100, 0, 0, 0, 0, 0) == 0 && sys(SYS_quotactl, 0x80000700, (i64)"/", 0, (i64)buf, 0, 0) == -ESRCH, "quotactl: Q_SYNC, and no quotas");
    check(sys(SYS_kexec_load, 0, 0, 0, 0, 0, 0) == -EPERM && sys(SYS_init_module, 0, 0, 0, 0, 0, 0) == -EPERM && sys(SYS_acct, 0, 0, 0, 0, 0, 0) == -EPERM &&
          sys(SYS_bpf, 0, 0, 0, 0, 0, 0) == -EPERM && sys(SYS_perf_event_open, (i64)buf, 0, -1, -1, 0, 0) == -EPERM && sys(SYS_vhangup, 0, 0, 0, 0, 0, 0) == 0,
          "kexec, modules, accounting, BPF and perf: EPERM; vhangup with no terminal");
    u64 ids[4] = {0};
    u32 size = sizeof ids, small = 4;
    check(sys(SYS_lsm_list_modules, (i64)ids, (i64)&size, 0, 0, 0, 0) == 1 && ids[0] == 100 && size == 8 && sys(SYS_lsm_list_modules, (i64)ids, (i64)&small, 0, 0, 0, 0) == -E2BIG &&
          small == 8, "lsm_list_modules: capability alone; E2BIG with the size it needs");
    size = sizeof buf;
    check(sys(SYS_lsm_get_self_attr, 100, (i64)buf, (i64)&size, 0, 0, 0) == 0 && size == 0 && sys(SYS_lsm_set_self_attr, 100, (i64)buf, 16, 0, 0, 0) == -EOPNOTSUPP &&
          sys(SYS_landlock_create_ruleset, 0, 0, 1, 0, 0, 0) == -EOPNOTSUPP, "no LSM attributes to get or set; Landlock off");
#if !defined(__aarch64__)
    check(sys(SYS_iopl, 0, 0, 0, 0, 0, 0) == 0 && sys(SYS_iopl, 3, 0, 0, 0, 0, 0) == -EPERM && sys(SYS_iopl, 4, 0, 0, 0, 0, 0) == -EINVAL &&
          sys(SYS_ioperm, 0x3f8, 8, 1, 0, 0, 0) == -EPERM && sys(SYS_ioperm, 0x3f8, 8, 0, 0, 0, 0) == 0 && sys(SYS_ioperm, 0xfff8, 16, 1, 0, 0, 0) == -EINVAL,
          "iopl and ioperm: lowering, nothing to raise");
#endif
    (void)pid;
    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
