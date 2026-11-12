// ptrace and process_vm_readv/writev, for lambda. A child that asks to be traced (PTRACE_TRACEME) stops at its own
// SIGSTOP: its siginfo, peeking and poking its memory, process_vm_readv/writev (and their EFAULT, EINVAL and ESRCH),
// /proc's TracerPid and tracing stop. Then each system call's entry and exit (PTRACE_O_TRACESYSGOOD): what
// PTRACE_GET_SYSCALL_INFO says, the call in orig_rax or x8 (and ARM64's x7 marking entry and exit), a result changed
// at the exit, a call skipped at its entry with its result set. Signal-delivery stops: one suppressed, one passed on
// to its handler. PTRACE_O_TRACEFORK's event, with the new process traced from its SIGSTOP and its end heard of by
// the tracer, which isn't its parent. PTRACE_O_TRACEEXIT's event with the status, then the end. A second child
// seized: EPERM to seize it again, PTRACE_INTERRUPT's PTRACE_EVENT_STOP, PEEKUSER and GETREGS (x86-64's; EIO on
// ARM64), a poke to its code (read-only to it), a group stop (EVENT_STOP, no siginfo) left by PTRACE_LISTEN until
// SIGCONT, and PTRACE_DETACH. A third attached with SIGSTOP, then PTRACE_KILL. Freestanding, for ARM64 and x86-64
// alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned char u8;

#if defined(__aarch64__)
enum { SYS_openat = 56, SYS_close = 57, SYS_read = 63, SYS_write = 64, SYS_exit_group = 94, SYS_nanosleep = 101,
       SYS_ptrace = 117, SYS_kill = 129, SYS_rt_sigaction = 134, SYS_getpid = 172, SYS_getppid = 173,
       SYS_clone = 220, SYS_wait4 = 260, SYS_process_vm_readv = 270, SYS_process_vm_writev = 271 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define X86 0
#define AUDIT_ARCH 0xc00000b7u
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_rt_sigaction = 13, SYS_nanosleep = 35, SYS_getpid = 39,
       SYS_clone = 56, SYS_exit_group = 231, SYS_wait4 = 61, SYS_kill = 62, SYS_ptrace = 101, SYS_getppid = 110,
       SYS_openat = 257, SYS_process_vm_readv = 310, SYS_process_vm_writev = 311 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define X86 1
#define AUDIT_ARCH 0xc000003eu
#endif

#define EPERM 1
#define ESRCH 3
#define EIO 5
#define EFAULT 14
#define EINVAL 22
#define ENOSYS 38
#define SIGKILL 9
#define SIGUSR1 10
#define SIGUSR2 12
#define SIGTRAP 5
#define SIGCHLD 17
#define SIGCONT 18
#define SIGSTOP 19
#define WALL 0x40000000

enum { TRACEME = 0, PEEKTEXT = 1, PEEKDATA = 2, PEEKUSR = 3, POKETEXT = 4, POKEDATA = 5, CONT = 7, KILL = 8,
       SINGLESTEP = 9, GETREGS = 12, ATTACH = 16, DETACH = 17, SYSCALL = 24, SETOPTIONS = 0x4200,
       GETEVENTMSG = 0x4201, GETSIGINFO = 0x4202, GETREGSET = 0x4204, SETREGSET = 0x4205, SEIZE = 0x4206,
       INTERRUPT = 0x4207, LISTEN = 0x4208, GET_SYSCALL_INFO = 0x420e };
#define O_TRACESYSGOOD 1
#define O_TRACEFORK 2
#define O_TRACEEXIT 0x40
#define EVENT_FORK 1
#define EVENT_EXIT 6
#define EVENT_STOP 128
#define NT_PRSTATUS 1
#define NT_ARM_SYSTEM_CALL 0x404

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
static int has(const char *hay, i64 n, const char *needle) {
    u64 m = len(needle);
    for (i64 i = 0; i + (i64)m <= n; i++) if (same(hay + i, needle, m)) return 1;
    return 0;
}
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }
static i64 pt(i64 req, i64 pid, i64 addr, i64 data) { return sys(SYS_ptrace, req, pid, addr, data, 0, 0); }
static int wait_for(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, WALL, 0, 0, 0); return status; }
static int stopped_by(int st) { return (st & 0xff) == 0x7f ? (st >> 8) & 0xff : -1; }
static int event_of(int st) { return (st >> 16) & 0xff; }
static void nap(void) { i64 ts[2] = {0, 5000000}; sys(SYS_nanosleep, (i64)ts, 0, 0, 0, 0, 0); }
static char *num(char *at, i64 n) {
    char d[24]; int k = 0;
    do { d[k++] = '0' + n % 10; n /= 10; } while (n);
    while (k) *at++ = d[--k];
    return at;
}
static void cat(char **at, const char *s) { while (*s) *(*at)++ = *s++; }
static void handle(int sig, void (*handler)(int)) {
    u64 action[4] = {(u64)handler, 0, 0, 0};
    sys(SYS_rt_sigaction, sig, (i64)action, 0, 8, 0, 0);
}

// The traced child's: what the tracer peeks and pokes, and what it found, for the tracer to peek at its end.
static volatile u64 secret = 1;
static char message[32];
static char message2[32];
static volatile i64 r_getppid, r_getpid, grand_status;
static volatile int got_usr1, got_usr2;
static void on_usr(int sig) { if (sig == SIGUSR1) got_usr1 = got_usr1 + 1; else got_usr2 = got_usr2 + 1; }

static void traced_child(void) {
    pt(TRACEME, 0, 0, 0);
    secret = 0x1122334455667788;
    for (int i = 0; i < 20; i++) message[i] = "hello from the child"[i];
    sys(SYS_kill, sys(SYS_getpid, 0, 0, 0, 0, 0, 0), SIGSTOP, 0, 0, 0, 0);
    r_getppid = sys(SYS_getppid, 0, 0, 0, 0, 0, 0);
    r_getpid = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    handle(SIGUSR1, on_usr);
    handle(SIGUSR2, on_usr);
    i64 me = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    sys(SYS_kill, me, SIGUSR1, 0, 0, 0, 0);
    sys(SYS_kill, me, SIGUSR2, 0, 0, 0, 0);
    i64 g = fork();
    if (g == 0) quit(3);
    int st = -1;
    sys(SYS_wait4, g, (i64)&st, 0, 0, 0, 0);
    grand_status = st;
    quit(42);
}

static void spinner(void) { for (;;) nap(); }

// The registers as NT_PRSTATUS has them: 27 on x86-64, 34 on ARM64.
static u64 regs[34];
static i64 getregs(i64 pid) { u64 iov[2] = {(u64)regs, sizeof regs}; return pt(GETREGSET, pid, NT_PRSTATUS, (i64)iov); }
static i64 setregs(i64 pid) { u64 iov[2] = {(u64)regs, X86 ? 27 * 8 : 34 * 8}; return pt(SETREGSET, pid, NT_PRSTATUS, (i64)iov); }
#define RESULT (X86 ? 10 : 0)
static u64 peek(i64 pid, const volatile void *at) { u64 w = 0; pt(PEEKDATA, pid, (i64)at, (i64)&w); return w; }

void cmain(i64 *sp) {
    (void)sp;
    i64 me = sys(SYS_getpid, 0, 0, 0, 0, 0, 0);
    char buf[512];
    u8 info[128];

    i64 pid = fork();
    if (pid == 0) traced_child();
    int st = wait_for(pid);
    check(stopped_by(st) == SIGSTOP, "PTRACE_TRACEME, then the child's own SIGSTOP: its tracer's wait sees the stop");
    check(pt(GETSIGINFO, pid, 0, (i64)info) == 0 && *(int *)info == SIGSTOP && *(int *)(info + 16) == pid, "PTRACE_GETSIGINFO: the signal, and who sent it");
    check(peek(pid, &secret) == 0x1122334455667788, "PTRACE_PEEKDATA reads the child's memory, not the parent's");
    check(pt(POKEDATA, pid, (i64)&secret, 0xabcdef) == 0 && peek(pid, &secret) == 0xabcdef && secret == 1, "PTRACE_POKEDATA writes it");
    u64 local[2] = {(u64)buf, 20}, remote[2] = {(u64)message, 20};
    check(sys(SYS_process_vm_readv, pid, (i64)local, 1, (i64)remote, 1, 0) == 20 && same(buf, "hello from the child", 20), "process_vm_readv");
    u64 l2[2] = {(u64)"written", 4}, l3[2] = {(u64)"written" + 4, 4}, r2[2] = {(u64)message2, 7};
    u64 lv[4] = {l2[0], l2[1], l3[0], 3};
    check(sys(SYS_process_vm_writev, pid, (i64)lv, 2, (i64)r2, 1, 0) == 7 && sys(SYS_process_vm_readv, pid, (i64)local, 1, (i64)r2, 1, 0) == 7 && same(buf, "written", 7),
          "process_vm_writev, from two ranges to one");
    u64 bad[2] = {0x10, 8};
    check(sys(SYS_process_vm_readv, pid, (i64)local, 1, (i64)bad, 1, 0) == -EFAULT && sys(SYS_process_vm_readv, pid, (i64)local, 1, (i64)remote, 1, 1) == -EINVAL &&
          sys(SYS_process_vm_readv, 999999, (i64)local, 1, (i64)remote, 1, 0) == -ESRCH, "process_vm_readv: EFAULT, EINVAL for flags, ESRCH");
    {
        char path[64], *at = path;
        cat(&at, "/proc/");
        at = num(at, pid);
        cat(&at, "/status");
        *at = 0;
        int fd = sys(SYS_openat, -100, (i64)path, 0, 0, 0, 0);
        i64 n = sys(SYS_read, fd, (i64)buf, sizeof buf, 0, 0, 0);
        sys(SYS_close, fd, 0, 0, 0, 0, 0);
        char want[32];
        at = want;
        cat(&at, "TracerPid:\t");
        at = num(at, me);
        *at = 0;
        check(has(buf, n, want) && has(buf, n, "t (tracing stop)"), "/proc/<pid>/status: TracerPid, and t (tracing stop)");
    }

    // Each system call's entry and exit.
    check(pt(SETOPTIONS, pid, 0, O_TRACESYSGOOD | O_TRACEFORK | O_TRACEEXIT) == 0 && pt(SETOPTIONS, pid, 0, 1 << 30) == -EINVAL, "PTRACE_SETOPTIONS, and EINVAL for one Linux hasn't");
    pt(SYSCALL, pid, 0, 0);
    st = wait_for(pid);
    u8 si[88];
    i64 size = pt(GET_SYSCALL_INFO, pid, sizeof si, (i64)si);
    check(stopped_by(st) == (SIGTRAP | 0x80), "PTRACE_SYSCALL: a stop at the next call's entry, SIGTRAP | 0x80");
    check(size == 80 && si[0] == 1 && *(u32 *)(si + 4) == AUDIT_ARCH && *(u64 *)(si + 24) == SYS_getppid, "PTRACE_GET_SYSCALL_INFO at an entry: the call, for this arch");
    getregs(pid);
    check(X86 ? regs[15] == SYS_getppid && regs[10] == (u64)-ENOSYS : regs[8] == SYS_getppid && regs[7] == 0, X86 ? "at an entry: orig_rax is the call, rax -ENOSYS" : "at an entry: x8 is the call, x7 0");
    pt(SYSCALL, pid, 0, 0);
    st = wait_for(pid);
    size = pt(GET_SYSCALL_INFO, pid, sizeof si, (i64)si);
    check(stopped_by(st) == (SIGTRAP | 0x80) && size == 33 && si[0] == 2 && *(i64 *)(si + 24) == me && si[32] == 0, "at its exit: what it returned");
    getregs(pid);
    check(X86 || regs[7] == 1, "at an exit: x7 1 on ARM64");
    regs[RESULT] = 4242;
    check(setregs(pid) == 0, "PTRACE_SETREGSET at the exit, to change the result");
    pt(SYSCALL, pid, 0, 0);
    st = wait_for(pid);
    getregs(pid);
    if (X86) {
        regs[15] = (u64)-1;
        regs[10] = 77;
        setregs(pid);
    } else {
        int skip = -1;
        u64 iov[2] = {(u64)&skip, 4};
        regs[0] = 77;
        setregs(pid);
        pt(SETREGSET, pid, NT_ARM_SYSTEM_CALL, (i64)iov);
    }
    pt(SYSCALL, pid, 0, 0);
    st = wait_for(pid);
    size = pt(GET_SYSCALL_INFO, pid, sizeof si, (i64)si);
    check(stopped_by(st) == (SIGTRAP | 0x80) && si[0] == 2 && *(i64 *)(si + 24) == 77, "a call skipped at its entry (as -1) still stops at its exit, with the result set");

    // Signal-delivery stops.
    pt(CONT, pid, 0, 0);
    st = wait_for(pid);
    check(stopped_by(st) == SIGUSR1, "PTRACE_CONT: the next stop is a signal about to be delivered");
    pt(CONT, pid, 0, 0);
    st = wait_for(pid);
    check(stopped_by(st) == SIGUSR2, "suppressed with 0, and the next one");
    pt(CONT, pid, 0, SIGUSR2);
    st = wait_for(pid);
    u64 g = 0;
    check(stopped_by(st) == SIGTRAP && event_of(st) == EVENT_FORK && pt(GETEVENTMSG, pid, 0, (i64)&g) == 0 && g > 0, "PTRACE_O_TRACEFORK: an event stop, with the new pid");
    int gst = wait_for(g);
    check(stopped_by(gst) == SIGSTOP, "the new process is traced too, from its SIGSTOP");
    pt(CONT, g, 0, 0);
    pt(CONT, pid, 0, 0);
    gst = wait_for(g);
    check(stopped_by(gst) == SIGTRAP && event_of(gst) == EVENT_EXIT, "with its tracer's options: its exit event too");
    pt(CONT, g, 0, 0);
    gst = wait_for(g);
    check(gst == 3 << 8, "its tracer, not its parent, hears of its end");
    st = wait_for(pid);
    u64 status = 0;
    check(stopped_by(st) == SIGTRAP && event_of(st) == EVENT_EXIT && pt(GETEVENTMSG, pid, 0, (i64)&status) == 0 && status == 42 << 8, "PTRACE_O_TRACEEXIT: a stop with the status it ends with");
    check(peek(pid, &r_getppid) == 4242, "the child got the result its tracer set");
    check(peek(pid, &r_getpid) == 77, "and the skipped call's");
    check((u32)peek(pid, &got_usr1) == 0 && (u32)peek(pid, &got_usr2) == 1, "the suppressed signal's handler never ran; the one passed on did");
    check(peek(pid, &grand_status) == 3 << 8, "its own child's wait still reaped it");
    pt(CONT, pid, 0, 0);
    st = wait_for(pid);
    check(st == 42 << 8, "then it ends");

    // Seized.
    i64 p2 = fork();
    if (p2 == 0) spinner();
    nap();
    check(pt(SEIZE, p2, 0, O_TRACESYSGOOD) == 0 && pt(SEIZE, p2, 0, 0) == -EPERM, "PTRACE_SEIZE, and EPERM to seize it again");
    check(pt(CONT, p2, 0, 0) == -ESRCH, "a request for a tracee that isn't stopped: ESRCH");
    check(pt(INTERRUPT, p2, 0, 0) == 0, "PTRACE_INTERRUPT");
    st = wait_for(p2);
    check(stopped_by(st) == SIGTRAP && event_of(st) == EVENT_STOP, "it stops with PTRACE_EVENT_STOP");
    if (X86) {
        u64 r[27], rip = 0;
        check(pt(GETREGS, p2, 0, (i64)r) == 0 && pt(PEEKUSR, p2, 16 * 8, (i64)&rip) == 0 && rip == r[16] && rip != 0, "PTRACE_GETREGS, and PTRACE_PEEKUSER of rip");
    } else {
        u64 r[34];
        check(pt(GETREGS, p2, 0, (i64)r) == -EIO && pt(PEEKUSR, p2, 0, (i64)r) == -EIO, "PTRACE_GETREGS and PTRACE_PEEKUSER: EIO on ARM64");
    }
    u64 code = 0;
    check(pt(PEEKTEXT, p2, (i64)spinner, (i64)&code) == 0 && pt(POKETEXT, p2, (i64)spinner, code) == 0 && peek(p2, (const void *)spinner) == code, "PTRACE_POKETEXT to its code, which it can't write");
    check(pt(SINGLESTEP, p2, 0, 0) == -EIO, "PTRACE_SINGLESTEP: EIO (not yet)");
    sys(SYS_kill, p2, SIGSTOP, 0, 0, 0, 0);
    pt(CONT, p2, 0, 0);
    st = wait_for(p2);
    check(stopped_by(st) == SIGSTOP && event_of(st) == 0, "a stop signal: first its signal-delivery stop");
    pt(CONT, p2, 0, SIGSTOP);
    st = wait_for(p2);
    check(stopped_by(st) == SIGSTOP && event_of(st) == EVENT_STOP && pt(GETSIGINFO, p2, 0, (i64)info) == -EINVAL, "passed on: a group stop, PTRACE_EVENT_STOP with no siginfo");
    check(pt(LISTEN, p2, 0, 0) == 0, "PTRACE_LISTEN");
    sys(SYS_kill, p2, SIGCONT, 0, 0, 0, 0);
    st = wait_for(p2);
    check(stopped_by(st) == SIGCONT, "SIGCONT ends it, and is then delivered");
    pt(INTERRUPT, p2, 0, 0);
    pt(CONT, p2, 0, 0);
    st = wait_for(p2);
    check(event_of(st) == EVENT_STOP && pt(DETACH, p2, 0, 0) == 0 && pt(PEEKDATA, p2, (i64)&secret, (i64)&code) == -ESRCH, "PTRACE_DETACH, and then it isn't traced");
    sys(SYS_kill, p2, SIGKILL, 0, 0, 0, 0);
    st = wait_for(p2);
    check(st == SIGKILL, "it carried on until killed");

    // Attached.
    i64 p3 = fork();
    if (p3 == 0) spinner();
    check(pt(ATTACH, me, 0, 0) == -EPERM && pt(ATTACH, 999999, 0, 0) == -ESRCH, "PTRACE_ATTACH to itself EPERM, to no one ESRCH");
    check(pt(ATTACH, p3, 0, 0) == 0, "PTRACE_ATTACH");
    st = wait_for(p3);
    check(stopped_by(st) == SIGSTOP, "it stops with the SIGSTOP the attach sent");
    pt(KILL, p3, 0, 0);
    st = wait_for(p3);
    check(st == SIGKILL, "PTRACE_KILL");

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures ? 2 : 0);
}
