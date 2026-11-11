// Sockets, for lambda: Unix stream, datagram and seqpacket sockets, socketpair, SCM_RIGHTS, TCP and UDP over
// 127.0.0.1, sockets in poll and epoll, socket options, netlink, /proc/net, the machine's /etc files and its DNS
// server. Nothing here leaves the machine. Freestanding, for ARM64 and x86-64 alike. Built like hello.c.

typedef unsigned long u64;
typedef long i64;
typedef unsigned int u32;
typedef unsigned short u16;
typedef unsigned char u8;

#if defined(__aarch64__)
enum { SYS_ioctl = 29, SYS_epoll_create1 = 20, SYS_epoll_ctl = 21, SYS_epoll_pwait = 22, SYS_fcntl = 25,
       SYS_unlinkat = 35, SYS_openat = 56, SYS_close = 57, SYS_pipe2 = 59, SYS_read = 63, SYS_write = 64, SYS_ppoll = 73,
       SYS_newfstatat = 79, SYS_exit_group = 94, SYS_nanosleep = 101, SYS_getpid = 172, SYS_socket = 198,
       SYS_socketpair = 199, SYS_bind = 200, SYS_listen = 201, SYS_connect = 203, SYS_getsockname = 204,
       SYS_getpeername = 205, SYS_sendto = 206, SYS_recvfrom = 207, SYS_setsockopt = 208, SYS_getsockopt = 209,
       SYS_shutdown = 210, SYS_sendmsg = 211, SYS_recvmsg = 212, SYS_clone = 220, SYS_accept4 = 242,
       SYS_recvmmsg = 243, SYS_wait4 = 260, SYS_sendmmsg = 269 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 x8 __asm__("x8") = n, x0 __asm__("x0") = a, x1 __asm__("x1") = b, x2 __asm__("x2") = c,
                 x3 __asm__("x3") = d, x4 __asm__("x4") = e, x5 __asm__("x5") = f;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x8), "r"(x1), "r"(x2), "r"(x3), "r"(x4), "r"(x5) : "memory");
    return x0;
}
__asm__(".globl _start\n_start:\n mov x0, sp\n bl cmain\n b .\n");
#define ST_MODE 16
struct event { u32 events; u64 data; };
#else
enum { SYS_read = 0, SYS_write = 1, SYS_close = 3, SYS_ioctl = 16, SYS_nanosleep = 35, SYS_getpid = 39,
       SYS_socket = 41, SYS_connect = 42, SYS_accept = 43, SYS_sendto = 44, SYS_recvfrom = 45, SYS_sendmsg = 46,
       SYS_recvmsg = 47, SYS_shutdown = 48, SYS_bind = 49, SYS_listen = 50, SYS_getsockname = 51,
       SYS_getpeername = 52, SYS_socketpair = 53, SYS_setsockopt = 54, SYS_getsockopt = 55, SYS_clone = 56,
       SYS_wait4 = 61, SYS_fcntl = 72, SYS_exit_group = 231, SYS_epoll_ctl = 233, SYS_openat = 257,
       SYS_newfstatat = 262, SYS_unlinkat = 263, SYS_ppoll = 271, SYS_epoll_pwait = 281, SYS_accept4 = 288,
       SYS_epoll_create1 = 291, SYS_pipe2 = 293, SYS_recvmmsg = 299, SYS_sendmmsg = 307 };
static i64 sys(i64 n, i64 a, i64 b, i64 c, i64 d, i64 e, i64 f) {
    register i64 r10 __asm__("r10") = d, r8 __asm__("r8") = e, r9 __asm__("r9") = f;
    i64 ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10), "r"(r8), "r"(r9) : "rcx", "r11", "memory");
    return ret;
}
__asm__(".globl _start\n_start:\n xor %rbp, %rbp\n mov %rsp, %rdi\n and $-16, %rsp\n call cmain\n hlt\n");
#define ST_MODE 24
struct __attribute__((packed)) event { u32 events; u64 data; };
#endif

#define AT_FDCWD (-100)
#define AF_UNIX 1
#define AF_INET 2
#define AF_INET6 10
#define AF_NETLINK 16
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOCK_RAW 3
#define SOCK_SEQPACKET 5
#define SOCK_NONBLOCK 0x800
#define SOCK_CLOEXEC 0x80000
#define SOL_SOCKET 1
#define SO_REUSEADDR 2
#define SO_TYPE 3
#define SO_ERROR 4
#define SO_PEERCRED 17
#define SO_RCVTIMEO 20
#define SO_ACCEPTCONN 30
#define IPPROTO_TCP 6
#define TCP_NODELAY 1
#define SCM_RIGHTS 1
#define MSG_PEEK 2
#define MSG_TRUNC 0x20
#define MSG_DONTWAIT 0x40
#define MSG_NOSIGNAL 0x4000
#define SHUT_WR 1
#define FIONREAD 0x541b
#define SIOCGIFINDEX 0x8933
#define SIOCGIFADDR 0x8915
#define EPIPE 32
#define EAGAIN 11
#define ENOENT 2
#define ENXIO 6
#define EADDRINUSE 98
#define ECONNRESET 104
#define ENOTCONN 107
#define ECONNREFUSED 111
#define EINPROGRESS 115
#define EPERM 1
#define POLLIN 1
#define POLLOUT 4
#define POLLHUP 0x10
#define POLLRDHUP 0x2000
#define EPOLLIN 1
#define SIGCHLD 17
#define S_IFMT 0170000
#define S_IFSOCK 0140000

static u64 len(const char *s) { u64 n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { sys(SYS_write, 1, (i64)s, len(s), 0, 0, 0); }
static int failures;
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static int same(const void *a, const void *b, u64 n) { const u8 *x = a, *y = b; for (u64 i = 0; i < n; i++) if (x[i] != y[i]) return 0; return 1; }
static void copy(void *to, const void *from, u64 n) { u8 *t = to; const u8 *f = from; for (u64 i = 0; i < n; i++) t[i] = f[i]; }
static void zero(void *to, u64 n) { u8 *t = to; for (u64 i = 0; i < n; i++) t[i] = 0; }
static int has(const char *hay, i64 n, const char *needle) {
    u64 m = len(needle);
    for (i64 i = 0; i + (i64)m <= n; i++) if (same(hay + i, needle, m)) return 1;
    return 0;
}
static void sleep_ms(int ms) { i64 ts[2] = {0, ms * 1000000L}; sys(SYS_nanosleep, (i64)ts, 0, 0, 0, 0, 0); }
static i64 fork(void) { return sys(SYS_clone, SIGCHLD, 0, 0, 0, 0, 0); }
static int wait_for(i64 pid) { int status = -1; sys(SYS_wait4, pid, (i64)&status, 0, 0, 0, 0); return status; }
static void quit(int code) { sys(SYS_exit_group, code, 0, 0, 0, 0, 0); }

static i64 sock(int domain, int type) { return sys(SYS_socket, domain, type, 0, 0, 0, 0); }
static i64 send_s(int fd, const char *s) { return sys(SYS_sendto, fd, (i64)s, len(s), MSG_NOSIGNAL, 0, 0); }
static i64 recv_into(int fd, char *buf, u64 n, int flags) { return sys(SYS_recvfrom, fd, (i64)buf, n, flags, 0, 0); }
static i64 wr(int fd, const char *s) { return sys(SYS_write, fd, (i64)s, len(s), 0, 0, 0); }
static i64 rd(int fd, char *buf, u64 n) { return sys(SYS_read, fd, (i64)buf, n, 0, 0, 0); }

struct sockaddr_in { u16 family; u16 port; u8 addr[4]; u8 zero[8]; };
struct sockaddr_un { u16 family; char path[108]; };
static u16 be16(u16 v) { return (u16)(v << 8 | v >> 8); }
static struct sockaddr_in in4(u8 a, u8 b, u8 c, u8 d, u16 port) {
    struct sockaddr_in s; zero(&s, sizeof s); s.family = AF_INET; s.port = be16(port);
    s.addr[0] = a; s.addr[1] = b; s.addr[2] = c; s.addr[3] = d; return s;
}
static u64 un(struct sockaddr_un *s, const char *path) {
    zero(s, sizeof *s); s->family = AF_UNIX; copy(s->path, path, len(path)); return 2 + len(path) + 1;
}
struct iovec { void *base; u64 len; };
struct msghdr { void *name; u32 namelen; u32 pad; struct iovec *iov; u64 iovlen; void *control; u64 controllen; int flags; int pad2; };
struct mmsghdr { struct msghdr hdr; u32 len; u32 pad; };
struct pollfd { int fd; short events, revents; };
static int poll1(int fd, short events, int ms) {
    struct pollfd p = {fd, events, 0};
    i64 ts[2] = {ms / 1000, ms % 1000 * 1000000L};
    i64 n = sys(SYS_ppoll, (i64)&p, 1, (i64)ts, 0, 8, 0);
    return n == 1 ? p.revents : (int)n;
}
static int int_opt(int fd, int level, int name) {
    int v = -1; u32 l = 4;
    sys(SYS_getsockopt, fd, level, name, (i64)&v, (i64)&l, 0);
    return v;
}

void cmain(i64 *sp) {
    (void)sp;
    char buf[512];
    i64 n;

    // socketpair: a stream both ways, a byte at a time or all at once; MSG_PEEK and FIONREAD.
    int sv[2] = {-1, -1};
    check(sys(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, (i64)sv, 0, 0) == 0 && sv[0] >= 0 && sv[1] > sv[0], "socketpair makes two");
    send_s(sv[0], "hello");
    int avail = -1;
    sys(SYS_ioctl, sv[1], FIONREAD, (i64)&avail, 0, 0, 0);
    n = recv_into(sv[1], buf, 2, MSG_PEEK);
    check(avail == 5 && n == 2 && same(buf, "he", 2), "socketpair: FIONREAD and MSG_PEEK");
    n = rd(sv[1], buf, sizeof buf);
    check(n == 5 && same(buf, "hello", 5), "socketpair: read what was sent");
    wr(sv[1], "back");
    n = rd(sv[0], buf, sizeof buf);
    check(n == 4 && same(buf, "back", 4), "socketpair: and the other way, with write and read");
    check(recv_into(sv[0], buf, 8, MSG_DONTWAIT) == -EAGAIN, "socketpair: nothing waiting is EAGAIN");
    sys(SYS_shutdown, sv[1], SHUT_WR, 0, 0, 0, 0);
    check(rd(sv[0], buf, 8) == 0 && (poll1(sv[0], POLLIN | POLLRDHUP, 0) & POLLRDHUP), "shutdown(SHUT_WR): the other end reads the end, and poll says POLLRDHUP");
    sys(SYS_close, sv[1], 0, 0, 0, 0, 0);
    check(send_s(sv[0], "x") == -EPIPE && (poll1(sv[0], POLLIN, 0) & POLLHUP), "writing to a closed peer is EPIPE; poll says POLLHUP");
    sys(SYS_close, sv[0], 0, 0, 0, 0, 0);

    // SCM_RIGHTS: a pipe's read end passed across, read from on the other side.
    sys(SYS_socketpair, AF_UNIX, SOCK_STREAM, 0, (i64)sv, 0, 0);
    int pipe_fds[2];
    sys(SYS_pipe2, (i64)pipe_fds, 0, 0, 0, 0, 0);
    wr(pipe_fds[1], "through the pipe");
    {
        struct iovec iov = {"F", 1};
        u64 control[3] = {16 + 4, (u64)SCM_RIGHTS << 32 | SOL_SOCKET, (u64)(u32)pipe_fds[0]};
        struct msghdr m; zero(&m, sizeof m);
        m.iov = &iov; m.iovlen = 1; m.control = control; m.controllen = 24;
        n = sys(SYS_sendmsg, sv[0], (i64)&m, 0, 0, 0, 0);
    }
    sys(SYS_close, pipe_fds[0], 0, 0, 0, 0, 0);
    {
        char c = 0;
        struct iovec iov = {&c, 1};
        u64 control[4] = {0, 0, 0, 0};
        struct msghdr m; zero(&m, sizeof m);
        m.iov = &iov; m.iovlen = 1; m.control = control; m.controllen = sizeof control;
        i64 got = sys(SYS_recvmsg, sv[1], (i64)&m, 0, 0, 0, 0);
        int passed = (int)(control[2] & 0xffffffff);
        int level = (int)(control[1] & 0xffffffff), type = (int)(control[1] >> 32);
        int ok = n == 1 && got == 1 && c == 'F' && m.controllen == 24 && level == SOL_SOCKET && type == SCM_RIGHTS && passed >= 0;
        i64 r = ok ? rd(passed, buf, sizeof buf) : -1;
        check(ok && r == 16 && same(buf, "through the pipe", 16), "SCM_RIGHTS: a pipe passed over a socket, read from the other side");
    }
    sys(SYS_close, sv[0], 0, 0, 0, 0, 0);
    sys(SYS_close, sv[1], 0, 0, 0, 0, 0);

    // Seqpacket keeps each message; a datagram pair too.
    sys(SYS_socketpair, AF_UNIX, SOCK_SEQPACKET, 0, (i64)sv, 0, 0);
    send_s(sv[0], "first message");
    send_s(sv[0], "second");
    n = recv_into(sv[1], buf, 5, MSG_TRUNC);
    i64 n2 = recv_into(sv[1], buf, sizeof buf, 0);
    check(n == 13 && n2 == 6 && same(buf, "second", 6), "seqpacket: a message cut short is gone; MSG_TRUNC says how long it was");
    sys(SYS_close, sv[0], 0, 0, 0, 0, 0);
    sys(SYS_close, sv[1], 0, 0, 0, 0, 0);

    // A Unix stream server at a path, with a client in another process.
    struct sockaddr_un addr;
    u64 alen = un(&addr, "/tmp/net-test.sock");
    sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/net-test.sock", 0, 0, 0, 0);
    int server = sock(AF_UNIX, SOCK_STREAM);
    check(sys(SYS_bind, server, (i64)&addr, alen, 0, 0, 0) == 0 && sys(SYS_listen, server, 8, 0, 0, 0, 0) == 0, "unix: bound to a path and listening");
    {
        u64 st[32];
        int other = sock(AF_UNIX, SOCK_STREAM);
        int stat_ok = sys(SYS_newfstatat, AT_FDCWD, (i64)"/tmp/net-test.sock", (i64)st, 0, 0, 0) == 0 && ((*(u32 *)((char *)st + ST_MODE)) & S_IFMT) == S_IFSOCK;
        check(stat_ok && sys(SYS_bind, other, (i64)&addr, alen, 0, 0, 0) == -EADDRINUSE && sys(SYS_openat, AT_FDCWD, (i64)"/tmp/net-test.sock", 0, 0, 0, 0) == -ENXIO,
              "unix: the path is a socket file; binding it again is EADDRINUSE; opening it is ENXIO");
        sys(SYS_close, other, 0, 0, 0, 0, 0);
    }
    i64 child = fork();
    if (child == 0) {
        int c = sock(AF_UNIX, SOCK_STREAM);
        if (sys(SYS_connect, c, (i64)&addr, alen, 0, 0, 0) != 0) quit(1);
        send_s(c, "from the child");
        n = rd(c, buf, sizeof buf);
        quit(n == 5 && same(buf, "reply", 5) ? 0 : 2);
    }
    int conn = sys(SYS_accept4, server, 0, 0, SOCK_CLOEXEC, 0, 0);
    n = rd(conn, buf, sizeof buf);
    int cred[3] = {0, 0, 0};
    u32 cl = sizeof cred;
    sys(SYS_getsockopt, conn, SOL_SOCKET, SO_PEERCRED, (i64)cred, (i64)&cl, 0);
    check(conn >= 0 && n == 14 && same(buf, "from the child", 14) && cred[0] == child, "unix: accepted a connection from another process; SO_PEERCRED says which");
    {
        struct sockaddr_un got; u32 gl = sizeof got;
        sys(SYS_getsockname, conn, (i64)&got, (i64)&gl, 0, 0, 0);
        check(gl == alen && same(got.path, "/tmp/net-test.sock", 19), "unix: getsockname on the accepted end says the server's path");
    }
    send_s(conn, "reply");
    check(wait_for(child) == 0, "unix: the child read the reply");
    sys(SYS_close, conn, 0, 0, 0, 0, 0);
    sys(SYS_close, server, 0, 0, 0, 0, 0);
    {
        int c = sock(AF_UNIX, SOCK_STREAM);
        struct sockaddr_un missing; u64 ml = un(&missing, "/tmp/no-such.sock");
        check(sys(SYS_connect, c, (i64)&addr, alen, 0, 0, 0) == -ECONNREFUSED && sys(SYS_connect, c, (i64)&missing, ml, 0, 0, 0) == -ENOENT,
              "unix: a closed server's path is ECONNREFUSED; no file is ENOENT");
        sys(SYS_close, c, 0, 0, 0, 0, 0);
    }

    // Unix datagrams to a path, and an abstract name.
    {
        struct sockaddr_un log; u64 ll = un(&log, "/tmp/net-test.log");
        sys(SYS_unlinkat, AT_FDCWD, (i64)"/tmp/net-test.log", 0, 0, 0, 0);
        int r = sock(AF_UNIX, SOCK_DGRAM);
        sys(SYS_bind, r, (i64)&log, ll, 0, 0, 0);
        int s = sock(AF_UNIX, SOCK_DGRAM);
        struct sockaddr_un me; zero(&me, sizeof me); me.family = AF_UNIX; me.path[0] = 0; copy(me.path + 1, "net-test", 8);
        sys(SYS_bind, s, (i64)&me, 2 + 1 + 8, 0, 0, 0);
        sys(SYS_sendto, s, (i64)"a log line", 10, 0, (i64)&log, ll);
        struct sockaddr_un from; u32 fl = sizeof from;
        n = sys(SYS_recvfrom, r, (i64)buf, sizeof buf, 0, (i64)&from, (i64)&fl);
        check(n == 10 && same(buf, "a log line", 10) && fl == 11 && from.path[0] == 0 && same(from.path + 1, "net-test", 8),
              "unix datagrams: sent to a path, from an abstract name, which recvfrom says");
        sys(SYS_close, r, 0, 0, 0, 0, 0);
        sys(SYS_close, s, 0, 0, 0, 0, 0);
    }

    // TCP over 127.0.0.1: a server in another process, an echo, half-close.
    int tcp = sock(AF_INET, SOCK_STREAM);
    int one = 1;
    sys(SYS_setsockopt, tcp, SOL_SOCKET, SO_REUSEADDR, (i64)&one, 4, 0);
    struct sockaddr_in local = in4(127, 0, 0, 1, 8080);
    check(sys(SYS_bind, tcp, (i64)&local, 16, 0, 0, 0) == 0 && sys(SYS_listen, tcp, 16, 0, 0, 0, 0) == 0 && int_opt(tcp, SOL_SOCKET, SO_ACCEPTCONN) == 1,
          "tcp: listening on 127.0.0.1:8080");
    {
        int fd = sys(SYS_openat, AT_FDCWD, (i64)"/proc/net/tcp", 0, 0, 0, 0);
        n = fd >= 0 ? rd(fd, buf, sizeof buf) : -1;
        check(n > 0 && has(buf, n, "0100007F:1F90 00000000:0000 0A"), "/proc/net/tcp lists the listener");
        sys(SYS_close, fd, 0, 0, 0, 0, 0);
    }
    child = fork();
    if (child == 0) {
        int c = sys(SYS_accept4, tcp, 0, 0, 0, 0, 0);
        char b[64];
        i64 total = 0, got;
        while ((got = rd(c, b + total, sizeof b - total)) > 0) total += got;
        sys(SYS_sendto, c, (i64)b, total, 0, 0, 0);
        sys(SYS_close, c, 0, 0, 0, 0, 0);
        quit(total == 11 ? 0 : 3);
    }
    int client = sock(AF_INET, SOCK_STREAM);
    check(sys(SYS_connect, client, (i64)&local, 16, 0, 0, 0) == 0, "tcp: connected to 127.0.0.1:8080");
    {
        struct sockaddr_in me, peer; u32 ml = 16, pl = 16;
        sys(SYS_getsockname, client, (i64)&me, (i64)&ml, 0, 0, 0);
        sys(SYS_getpeername, client, (i64)&peer, (i64)&pl, 0, 0, 0);
        check(ml == 16 && me.addr[0] == 127 && be16(me.port) >= 32768 && same(&peer, &local, 8), "tcp: getsockname and getpeername");
    }
    sys(SYS_setsockopt, client, IPPROTO_TCP, TCP_NODELAY, (i64)&one, 4, 0);
    check(int_opt(client, IPPROTO_TCP, TCP_NODELAY) == 1 && int_opt(client, SOL_SOCKET, SO_TYPE) == SOCK_STREAM, "tcp: TCP_NODELAY and SO_TYPE");
    send_s(client, "hello ");
    send_s(client, "world");
    sys(SYS_shutdown, client, SHUT_WR, 0, 0, 0, 0);
    i64 total = 0;
    while ((n = rd(client, buf + total, sizeof buf - total)) > 0) total += n;
    check(total == 11 && same(buf, "hello world", 11) && wait_for(child) == 0, "tcp: an echo, after half-closing, to the end");
    sys(SYS_close, client, 0, 0, 0, 0, 0);

    // Refused, reset, non-blocking connect and accept, and epoll on sockets.
    {
        int c = sock(AF_INET, SOCK_STREAM);
        struct sockaddr_in closed = in4(127, 0, 0, 1, 8081);
        check(sys(SYS_connect, c, (i64)&closed, 16, 0, 0, 0) == -ECONNREFUSED, "tcp: nothing listening is ECONNREFUSED");
        sys(SYS_close, c, 0, 0, 0, 0, 0);
    }
    {
        int ns = sock(AF_INET, SOCK_STREAM | SOCK_NONBLOCK);
        sys(SYS_setsockopt, ns, SOL_SOCKET, SO_REUSEADDR, (i64)&one, 4, 0);
        struct sockaddr_in a = in4(0, 0, 0, 0, 9090);
        sys(SYS_bind, ns, (i64)&a, 16, 0, 0, 0);
        sys(SYS_listen, ns, 4, 0, 0, 0, 0);
        check(sys(SYS_accept4, ns, 0, 0, 0, 0, 0) == -EAGAIN, "tcp: a non-blocking accept with nobody there is EAGAIN");
        int ep = sys(SYS_epoll_create1, 0, 0, 0, 0, 0, 0);
        struct event e = {EPOLLIN, 77};
        sys(SYS_epoll_ctl, ep, 1, ns, (i64)&e, 0, 0);
        int c = sock(AF_INET, SOCK_STREAM | SOCK_NONBLOCK);
        // The machine's own address reaches a listener on any address.
        int fd = sys(SYS_socket, AF_INET, SOCK_DGRAM, 0, 0, 0, 0);
        u8 ifreq[40]; zero(ifreq, 40); copy(ifreq, "eth0", 4);
        sys(SYS_ioctl, fd, SIOCGIFADDR, (i64)ifreq, 0, 0, 0);
        struct sockaddr_in own = in4(ifreq[20], ifreq[21], ifreq[22], ifreq[23], 9090);
        sys(SYS_close, fd, 0, 0, 0, 0, 0);
        i64 r = sys(SYS_connect, c, (i64)&own, 16, 0, 0, 0);
        struct event got[4];
        i64 ready = sys(SYS_epoll_pwait, ep, (i64)got, 4, 1000, 0, 8);
        int a2 = sys(SYS_accept4, ns, 0, 0, SOCK_NONBLOCK, 0, 0);
        check(ifreq[20] == 10 && (r == 0 || r == -EINPROGRESS) && ready == 1 && got[0].data == 77 && a2 >= 0, "tcp: to the machine's own address; epoll says the listener's ready");
        check((poll1(c, POLLOUT, 1000) & POLLOUT) && int_opt(c, SOL_SOCKET, SO_ERROR) == 0, "tcp: a non-blocking connect becomes writable, with no SO_ERROR");
        send_s(c, "unread");
        sleep_ms(10);
        sys(SYS_close, a2, 0, 0, 0, 0, 0);
        check(recv_into(c, buf, 8, 0) == -ECONNRESET, "tcp: closing with data unread resets the other end");
        sys(SYS_close, c, 0, 0, 0, 0, 0);
        sys(SYS_close, ns, 0, 0, 0, 0, 0);
        sys(SYS_close, ep, 0, 0, 0, 0, 0);
    }
    sys(SYS_close, tcp, 0, 0, 0, 0, 0);

    // UDP over loopback, with recvmmsg and sendmmsg; SO_RCVTIMEO.
    {
        int a = sock(AF_INET, SOCK_DGRAM);
        struct sockaddr_in at = in4(127, 0, 0, 1, 5353);
        sys(SYS_bind, a, (i64)&at, 16, 0, 0, 0);
        int b = sock(AF_INET, SOCK_DGRAM);
        sys(SYS_sendto, b, (i64)"ping", 4, 0, (i64)&at, 16);
        struct sockaddr_in from; u32 fl = 16;
        n = sys(SYS_recvfrom, a, (i64)buf, sizeof buf, 0, (i64)&from, (i64)&fl);
        struct sockaddr_in bl; u32 bll = 16;
        sys(SYS_getsockname, b, (i64)&bl, (i64)&bll, 0, 0, 0);
        check(n == 4 && same(buf, "ping", 4) && from.addr[0] == 127 && from.port == bl.port, "udp: a datagram, and who sent it");
        char m1[] = "one", m2[] = "two";
        struct iovec v1 = {m1, 3}, v2 = {m2, 3};
        struct mmsghdr out[2]; zero(out, sizeof out);
        out[0].hdr.name = &at; out[0].hdr.namelen = 16; out[0].hdr.iov = &v1; out[0].hdr.iovlen = 1;
        out[1].hdr.name = &at; out[1].hdr.namelen = 16; out[1].hdr.iov = &v2; out[1].hdr.iovlen = 1;
        i64 sent = sys(SYS_sendmmsg, b, (i64)out, 2, 0, 0, 0);
        char r1[8], r2[8];
        struct iovec w1 = {r1, 8}, w2 = {r2, 8};
        struct mmsghdr in[2]; zero(in, sizeof in);
        in[0].hdr.iov = &w1; in[0].hdr.iovlen = 1; in[1].hdr.iov = &w2; in[1].hdr.iovlen = 1;
        i64 got = sys(SYS_recvmmsg, a, (i64)in, 2, 0, 0, 0);
        check(sent == 2 && got == 2 && in[0].len == 3 && same(r1, "one", 3) && same(r2, "two", 3), "udp: sendmmsg and recvmmsg");
        i64 tv[2] = {0, 50000};
        sys(SYS_setsockopt, a, SOL_SOCKET, SO_RCVTIMEO, (i64)tv, 16, 0);
        check(recv_into(a, buf, 8, 0) == -EAGAIN, "udp: SO_RCVTIMEO ends a wait with EAGAIN");
        sys(SYS_close, a, 0, 0, 0, 0, 0);
        sys(SYS_close, b, 0, 0, 0, 0, 0);
    }

    // What isn't a machine's: raw sockets.
    check(sock(AF_INET, SOCK_RAW) == -EPERM, "raw sockets are EPERM");

    // Netlink: the links, by name.
    {
        int nl = sys(SYS_socket, AF_NETLINK, SOCK_RAW | SOCK_CLOEXEC, 0, 0, 0, 0);
        u32 req[8] = {32, 18 | (1 | 0x300) << 16, 1, 0, 0, 0, 0, 0};
        u64 kernel[2] = {AF_NETLINK, 0};
        sys(SYS_sendto, nl, (i64)req, 32, 0, (i64)kernel, 12);
        static char reply[4096];
        n = rd(nl, reply, sizeof reply);
        check(nl >= 0 && n > 0 && has(reply, n, "lo") && has(reply, n, "eth0"), "netlink: RTM_GETLINK lists lo and eth0");
        sys(SYS_close, nl, 0, 0, 0, 0, 0);
        int fd = sock(AF_INET, SOCK_DGRAM);
        u8 ifreq[40]; zero(ifreq, 40); copy(ifreq, "eth0", 4);
        sys(SYS_ioctl, fd, SIOCGIFINDEX, (i64)ifreq, 0, 0, 0);
        check(*(int *)(ifreq + 16) == 2, "SIOCGIFINDEX: eth0 is interface 2");
        sys(SYS_close, fd, 0, 0, 0, 0, 0);
    }

    // The machine's /etc files, and its DNS server answering its own name.
    {
        int fd = sys(SYS_openat, AT_FDCWD, (i64)"/etc/resolv.conf", 0, 0, 0, 0);
        n = fd >= 0 ? rd(fd, buf, sizeof buf - 1) : -1;
        sys(SYS_close, fd, 0, 0, 0, 0, 0);
        // "nameserver 10.N.0.1"
        int i = 0;
        while (i + 11 < n && !same(buf + i, "nameserver ", 11)) i++;
        u8 ip[4] = {0, 0, 0, 0};
        int part = 0;
        for (int j = i + 11; j < n && buf[j] != '\n' && part < 4; j++) {
            if (buf[j] == '.') part++;
            else ip[part] = (u8)(ip[part] * 10 + (buf[j] - '0'));
        }
        check(n > 0 && ip[0] == 10 && ip[3] == 1, "/etc/resolv.conf names the machine's gateway");
        fd = sys(SYS_openat, AT_FDCWD, (i64)"/etc/hosts", 0, 0, 0, 0);
        n = fd >= 0 ? rd(fd, buf, sizeof buf) : -1;
        sys(SYS_close, fd, 0, 0, 0, 0, 0);
        check(n > 0 && has(buf, n, "127.0.0.1\tlocalhost") && has(buf, n, "carbon"), "/etc/hosts has localhost and the machine's name");

        int d = sock(AF_INET, SOCK_DGRAM);
        struct sockaddr_in dns = in4(ip[0], ip[1], ip[2], ip[3], 53);
        u8 q[] = {0xab, 0xcd, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 6, 'c', 'a', 'r', 'b', 'o', 'n', 0, 0, 1, 0, 1};
        sys(SYS_sendto, d, (i64)q, sizeof q, 0, (i64)&dns, 16);
        int ready = poll1(d, POLLIN, 2000);
        u8 a[512];
        n = recv_into(d, (char *)a, sizeof a, 0);
        check(ready == POLLIN && n == (i64)sizeof q + 16 && a[0] == 0xab && (a[2] & 0x80) && a[7] == 1 && a[n - 4] == 10 && a[n - 1] == 2,
              "DNS: the gateway answers the machine's own name with its address");
        sys(SYS_close, d, 0, 0, 0, 0, 0);
    }

    say(failures ? "some checks failed\n" : "all passed\n");
    quit(failures);
}
