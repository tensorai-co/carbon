// Winsock, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it. Everything goes over the
// machine's loopback, so it needs no network.
//
// It checks: TCP's listen, connect, accept and their names; send, recv (and MSG_PEEK), FIONREAD; select and
// WSAPoll; non-blocking sockets (FIONBIO, WSAEWOULDBLOCK, a connect seen through select); a receive timeout; options;
// overlapped WSARecv finishing through an event and through a completion port, at once and later, with and without
// FILE_SKIP_COMPLETION_PORT_ON_SUCCESS; closesocket aborting what's under way; shutdown; UDP's sendto and recvfrom;
// ConnectEx and AcceptEx from WSAIoctl, and GetAcceptExSockaddrs; getaddrinfo (numeric, passive, localhost, the
// machine's own name, and a name that numeric-only refuses); gethostname and inet_addr. ws2_32's oldest functions
// are imported by ordinal.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned short WCHAR;
typedef unsigned char BYTE;
typedef __UINTPTR_TYPE__ SOCKET;
typedef struct { unsigned short family, port; BYTE addr[4]; BYTE zero[8]; } SOCKADDR_IN;
typedef struct { DWORD len; char *buf; } WSABUF;
typedef struct { __UINTPTR_TYPE__ internal, internal_high; DWORD offset, offset_high; HANDLE event; } OVERLAPPED;
typedef struct { unsigned count; SOCKET fds[64]; } FD_SET;
typedef struct { int sec, usec; } TIMEVAL;
typedef struct { SOCKET fd; short events, revents; } WSAPOLLFD;
typedef struct addrinfo { int flags, family, socktype, protocol; __SIZE_TYPE__ addrlen; char *canonname; SOCKADDR_IN *addr; struct addrinfo *next; } ADDRINFOA;
typedef struct { DWORD a; unsigned short b, c; BYTE d[8]; } GUID;
typedef int(__stdcall *CONNECTEX)(SOCKET, const void *, int, void *, DWORD, DWORD *, OVERLAPPED *);
typedef int(__stdcall *ACCEPTEX)(SOCKET, SOCKET, void *, DWORD, DWORD, DWORD, DWORD *, OVERLAPPED *);
typedef void(__stdcall *GETACCEPTEXSOCKADDRS)(void *, DWORD, DWORD, DWORD, SOCKADDR_IN **, int *, SOCKADDR_IN **, int *);

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K DWORD __stdcall GetLastError(void);
K HANDLE __stdcall CreateEventW(void *, int, int, const WCHAR *);
K DWORD __stdcall WaitForSingleObject(HANDLE, DWORD);
K HANDLE __stdcall CreateIoCompletionPort(HANDLE, HANDLE, __UINTPTR_TYPE__, DWORD);
K int __stdcall GetQueuedCompletionStatus(HANDLE, DWORD *, __UINTPTR_TYPE__ *, OVERLAPPED **, DWORD);
K int __stdcall SetFileCompletionNotificationModes(HANDLE, BYTE);
K int __stdcall WSAStartup(unsigned short, void *);
K int __stdcall WSACleanup(void);
K int __stdcall WSAGetLastError(void);
K SOCKET __stdcall socket(int, int, int);
K SOCKET __stdcall WSASocketW(int, int, int, void *, unsigned, DWORD);
K int __stdcall bind(SOCKET, const void *, int);
K int __stdcall listen(SOCKET, int);
K int __stdcall connect(SOCKET, const void *, int);
K SOCKET __stdcall accept(SOCKET, void *, int *);
K int __stdcall getsockname(SOCKET, void *, int *);
K int __stdcall getpeername(SOCKET, void *, int *);
K int __stdcall send(SOCKET, const char *, int, int);
K int __stdcall recv(SOCKET, char *, int, int);
K int __stdcall sendto(SOCKET, const char *, int, int, const void *, int);
K int __stdcall recvfrom(SOCKET, char *, int, int, void *, int *);
K int __stdcall closesocket(SOCKET);
K int __stdcall shutdown(SOCKET, int);
K int __stdcall ioctlsocket(SOCKET, long, unsigned long *);
K int __stdcall setsockopt(SOCKET, int, int, const char *, int);
K int __stdcall getsockopt(SOCKET, int, int, char *, int *);
K int __stdcall select(int, FD_SET *, FD_SET *, FD_SET *, const TIMEVAL *);
K int __stdcall WSAPoll(WSAPOLLFD *, unsigned long, int);
K int __stdcall WSARecv(SOCKET, WSABUF *, DWORD, DWORD *, DWORD *, OVERLAPPED *, void *);
K int __stdcall WSASend(SOCKET, WSABUF *, DWORD, DWORD *, DWORD, OVERLAPPED *, void *);
K int __stdcall WSAGetOverlappedResult(SOCKET, OVERLAPPED *, DWORD *, int, DWORD *);
K int __stdcall WSAIoctl(SOCKET, DWORD, void *, DWORD, void *, DWORD, DWORD *, OVERLAPPED *, void *);
K int __stdcall getaddrinfo(const char *, const char *, const ADDRINFOA *, ADDRINFOA **);
K void __stdcall freeaddrinfo(ADDRINFOA *);
K int __stdcall gethostname(char *, int);
K unsigned long __stdcall inet_addr(const char *);
K unsigned short __stdcall htons(unsigned short);

#define INVALID ((SOCKET)~(__UINTPTR_TYPE__)0)
#define SOCKET_ERROR (-1)
#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define FIONBIO ((long)0x8004667e)
#define FIONREAD ((long)0x4004667f)

static HANDLE out;
static int failures;
static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void say(const char *s) { DWORD n; WriteFile(out, s, len(s), &n, 0); }
static void check(int ok, const char *what) {
    say(ok ? "ok      " : "FAILED  ");
    say(what);
    say("\n");
    if (!ok) { __asm__ volatile("" ::: "memory"); failures = failures + 1; }
}
static int same(const char *a, const char *b, int n) { for (int i = 0; i < n; i++) if (a[i] != b[i]) return 0; return 1; }
static void zero(void *p, unsigned n) { volatile BYTE *b = p; for (unsigned i = 0; i < n; i++) b[i] = 0; }

static BYTE wsa[408];
static char buf[256];
static SOCKADDR_IN loopback;
// fd_sets are big: static, so nothing calls memset to clear them.
static FD_SET reads, writes;

/// A socket on 127.0.0.1, at a port of its own: its address in `at`.
static SOCKET bound(int type, SOCKADDR_IN *at) {
    SOCKET s = socket(AF_INET, type, 0);
    bind(s, &loopback, sizeof loopback);
    int n = sizeof *at;
    getsockname(s, at, &n);
    return s;
}

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
#if defined(__aarch64__)
    say("winsock, arm64\n");
#elif defined(__i386__)
    say("winsock, x86 (WoW64)\n");
#else
    say("winsock, x86-64\n");
#endif
    check(socket(AF_INET, SOCK_STREAM, 0) == INVALID && WSAGetLastError() == 10093, "socket before WSAStartup: WSANOTINITIALISED");
    check(WSAStartup(0x202, wsa) == 0, "WSAStartup");
    loopback.family = AF_INET;
    loopback.addr[0] = 127;
    loopback.addr[3] = 1;

    // ---- TCP over loopback ----
    SOCKADDR_IN at, name;
    SOCKET listener = bound(SOCK_STREAM, &at);
    check(listener != INVALID && at.port != 0 && at.addr[0] == 127, "socket, bind to port 0, and getsockname: the port given");
    check(listen(listener, 4) == 0, "listen");
    SOCKET client = socket(AF_INET, SOCK_STREAM, 0);
    check(connect(client, &at, sizeof at) == 0, "connect, blocking");
    int n = sizeof name;
    SOCKET server = accept(listener, &name, &n);
    SOCKADDR_IN mine;
    int m = sizeof mine;
    getsockname(client, &mine, &m);
    check(server != INVALID && n == 16 && name.port == mine.port, "accept: the peer is the client");
    m = sizeof mine;
    check(getpeername(client, &mine, &m) == 0 && mine.port == at.port, "getpeername");
    check(send(client, "hello", 5, 0) == 5, "send");
    unsigned long ready = 0;
    for (int i = 0; i < 100 && ready < 5; i++) ioctlsocket(server, FIONREAD, &ready);
    check(ready == 5, "ioctlsocket FIONREAD");
    check(recv(server, buf, sizeof buf, 2) == 5 && same(buf, "hello", 5), "recv with MSG_PEEK");
    check(recv(server, buf, sizeof buf, 0) == 5 && same(buf, "hello", 5), "recv");

    reads.count = 1;
    reads.fds[0] = server;
    TIMEVAL now = {0, 0};
    check(select(0, &reads, 0, 0, &now) == 0 && reads.count == 0, "select: nothing to read, at once");
    send(client, "x", 1, 0);
    reads.count = 1;
    reads.fds[0] = server;
    TIMEVAL second = {1, 0};
    check(select(0, &reads, 0, 0, &second) == 1 && reads.count == 1 && reads.fds[0] == server, "select: readable");
    recv(server, buf, sizeof buf, 0);
    WSAPOLLFD poll = {server, 0x100, 0};
    check(WSAPoll(&poll, 1, 0) == 0, "WSAPoll: nothing yet");
    send(client, "y", 1, 0);
    check(WSAPoll(&poll, 1, 1000) == 1 && (poll.revents & 0x100), "WSAPoll: POLLRDNORM");
    recv(server, buf, sizeof buf, 0);

    // ---- Options, timeouts, non-blocking ----
    int one = 1, value = 0, size = sizeof value;
    check(setsockopt(client, 6, 1, (char *)&one, 4) == 0 && getsockopt(client, 6, 1, (char *)&value, &size) == 0 && value == 1, "TCP_NODELAY set and read");
    size = sizeof value;
    check(getsockopt(client, 0xffff, 0x1008, (char *)&value, &size) == 0 && value == SOCK_STREAM, "SO_TYPE");
    int ms = 50;
    setsockopt(server, 0xffff, 0x1006, (char *)&ms, 4);
    check(recv(server, buf, sizeof buf, 0) == SOCKET_ERROR && WSAGetLastError() == 10060, "SO_RCVTIMEO: WSAETIMEDOUT");
    ms = 0;
    setsockopt(server, 0xffff, 0x1006, (char *)&ms, 4);
    unsigned long on = 1;
    check(ioctlsocket(server, FIONBIO, &on) == 0, "ioctlsocket FIONBIO");
    check(recv(server, buf, sizeof buf, 0) == SOCKET_ERROR && WSAGetLastError() == 10035, "recv, non-blocking: WSAEWOULDBLOCK");
    on = 0;
    ioctlsocket(server, FIONBIO, &on);
    SOCKET quick = socket(AF_INET, SOCK_STREAM, 0);
    on = 1;
    ioctlsocket(quick, FIONBIO, &on);
    int c = connect(quick, &at, sizeof at);
    check(c == 0 || WSAGetLastError() == 10035, "connect, non-blocking: done or WSAEWOULDBLOCK");
    writes.count = 1;
    writes.fds[0] = quick;
    check(select(0, 0, &writes, 0, &second) == 1 && writes.fds[0] == quick, "select: the connect done");
    SOCKET quick_server = accept(listener, 0, 0);
    closesocket(quick_server);
    closesocket(quick);

    // ---- Overlapped ----
    OVERLAPPED ov;
    zero(&ov, sizeof ov);
    ov.event = CreateEventW(0, 1, 0, 0);
    WSABUF wb = {sizeof buf, buf};
    DWORD got = 0, flags = 0;
    check(WSARecv(server, &wb, 1, &got, &flags, &ov, 0) == SOCKET_ERROR && WSAGetLastError() == 997, "WSARecv, overlapped: WSA_IO_PENDING");
    check(WaitForSingleObject(ov.event, 0) == 0x102, "its event unset while it's under way");
    send(client, "later", 5, 0);
    check(WaitForSingleObject(ov.event, 2000) == 0, "its event set when the data comes");
    check(WSAGetOverlappedResult(server, &ov, &got, 0, &flags) && got == 5 && same(buf, "later", 5), "WSAGetOverlappedResult: the bytes");

    HANDLE port = CreateIoCompletionPort((HANDLE)server, 0, 77, 0);
    check(port != 0, "CreateIoCompletionPort with a socket");
    zero(&ov, sizeof ov);
    check(WSARecv(server, &wb, 1, &got, &flags, &ov, 0) == SOCKET_ERROR && WSAGetLastError() == 997, "WSARecv on the port's socket: pending");
    WSABUF sb = {4, "port"};
    DWORD sent = 0;
    check(WSASend(client, &sb, 1, &sent, 0, 0, 0) == 0 && sent == 4, "WSASend, not overlapped");
    DWORD bytes = 0;
    __UINTPTR_TYPE__ key = 0;
    OVERLAPPED *done = 0;
    check(GetQueuedCompletionStatus(port, &bytes, &key, &done, 2000) && bytes == 4 && key == 77 && done == &ov && same(buf, "port", 4), "GetQueuedCompletionStatus: the receive's packet");
    send(client, "now", 3, 0);
    ready = 0;
    for (int i = 0; i < 100 && ready < 3; i++) ioctlsocket(server, FIONREAD, &ready);
    zero(&ov, sizeof ov);
    got = 0;
    check(WSARecv(server, &wb, 1, &got, &flags, &ov, 0) == 0 && got == 3, "WSARecv with data there: done at once");
    check(GetQueuedCompletionStatus(port, &bytes, &key, &done, 0) && bytes == 3, "and its packet queued anyway");
    check(SetFileCompletionNotificationModes((HANDLE)server, 1), "SetFileCompletionNotificationModes: skip the port on success");
    send(client, "skip", 4, 0);
    ready = 0;
    for (int i = 0; i < 100 && ready < 4; i++) ioctlsocket(server, FIONREAD, &ready);
    zero(&ov, sizeof ov);
    check(WSARecv(server, &wb, 1, &got, &flags, &ov, 0) == 0 && got == 4, "WSARecv done at once again");
    check(!GetQueuedCompletionStatus(port, &bytes, &key, &done, 0) && GetLastError() == 258, "with no packet this time");

    SOCKET doomed_client = socket(AF_INET, SOCK_STREAM, 0);
    connect(doomed_client, &at, sizeof at);
    SOCKET doomed = accept(listener, 0, 0);
    OVERLAPPED aborted;
    zero(&aborted, sizeof aborted);
    aborted.event = CreateEventW(0, 1, 0, 0);
    WSARecv(doomed, &wb, 1, &got, &flags, &aborted, 0);
    check(closesocket(doomed) == 0 && WaitForSingleObject(aborted.event, 1000) == 0 && aborted.internal == 0xc0000120, "closesocket: what's under way ends aborted");
    closesocket(doomed_client);

    check(shutdown(client, 1) == 0 && recv(server, buf, sizeof buf, 0) == 0, "shutdown SD_SEND: the other end reads its end");
    check(closesocket(client) == 0 && closesocket(server) == 0, "closesocket");
    check(closesocket((SOCKET)0x1230) == SOCKET_ERROR && WSAGetLastError() == 10038, "closesocket on what isn't one: WSAENOTSOCK");

    // ---- UDP ----
    SOCKADDR_IN a1, a2, from;
    SOCKET u1 = bound(SOCK_DGRAM, &a1), u2 = bound(SOCK_DGRAM, &a2);
    check(sendto(u1, "datagram", 8, 0, &a2, sizeof a2) == 8, "sendto");
    int from_len = sizeof from;
    check(recvfrom(u2, buf, sizeof buf, 0, &from, &from_len) == 8 && same(buf, "datagram", 8) && from.port == a1.port, "recvfrom: the data and who sent it");
    closesocket(u1);
    closesocket(u2);

    // ---- ConnectEx and AcceptEx ----
    GUID connect_ex = {0x25a207b9, 0xddf3, 0x4660, {0x8e, 0xe9, 0x76, 0xe5, 0x8c, 0x74, 0x06, 0x3e}};
    GUID accept_ex = {0xb5367df1, 0xcbac, 0x11cf, {0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92}};
    GUID sockaddrs = {0xb5367df2, 0xcbac, 0x11cf, {0x95, 0xca, 0x00, 0x80, 0x5f, 0x48, 0xa1, 0x92}};
    CONNECTEX ConnectEx = 0;
    ACCEPTEX AcceptEx = 0;
    GETACCEPTEXSOCKADDRS GetAcceptExSockaddrs = 0;
    DWORD returned = 0;
    check(WSAIoctl(listener, 0xc8000006, &connect_ex, 16, &ConnectEx, sizeof ConnectEx, &returned, 0, 0) == 0 && ConnectEx && returned == sizeof ConnectEx, "WSAIoctl: ConnectEx");
    WSAIoctl(listener, 0xc8000006, &accept_ex, 16, &AcceptEx, sizeof AcceptEx, &returned, 0, 0);
    WSAIoctl(listener, 0xc8000006, &sockaddrs, 16, &GetAcceptExSockaddrs, sizeof GetAcceptExSockaddrs, &returned, 0, 0);
    check(AcceptEx && GetAcceptExSockaddrs, "WSAIoctl: AcceptEx and GetAcceptExSockaddrs");
    SOCKET accepting = WSASocketW(AF_INET, SOCK_STREAM, 0, 0, 0, 1);
    static char accept_buffer[64 + 2 * 32];
    OVERLAPPED aov, cov;
    zero(&aov, sizeof aov);
    zero(&cov, sizeof cov);
    aov.event = CreateEventW(0, 1, 0, 0);
    cov.event = CreateEventW(0, 1, 0, 0);
    DWORD accepted_bytes = 0;
    check(!AcceptEx(listener, accepting, accept_buffer, 64, 32, 32, &accepted_bytes, &aov) && WSAGetLastError() == 997, "AcceptEx: pending");
    SOCKADDR_IN cat;
    SOCKET connecting = bound(SOCK_STREAM, &cat);
    DWORD first_sent = 0;
    int connected = ConnectEx(connecting, &at, sizeof at, "first", 5, &first_sent, &cov);
    check(connected || WSAGetLastError() == 997, "ConnectEx, with bytes to send");
    check(WaitForSingleObject(cov.event, 2000) == 0 && WSAGetOverlappedResult(connecting, &cov, &first_sent, 0, &flags) && first_sent == 5, "ConnectEx done, its bytes sent");
    check(WaitForSingleObject(aov.event, 2000) == 0 && WSAGetOverlappedResult(listener, &aov, &accepted_bytes, 0, &flags) && accepted_bytes == 5 && same(accept_buffer, "first", 5),
          "AcceptEx done, with the first bytes");
    SOCKADDR_IN *local = 0, *remote = 0;
    int local_len = 0, remote_len = 0;
    GetAcceptExSockaddrs(accept_buffer, 64, 32, 32, &local, &local_len, &remote, &remote_len);
    check(local && remote && local_len == 16 && remote_len == 16 && remote->port == cat.port && local->port == at.port, "GetAcceptExSockaddrs: both ends");
    check(send(accepting, "back", 4, 0) == 4 && recv(connecting, buf, sizeof buf, 0) == 4 && same(buf, "back", 4), "the accepted socket works");
    closesocket(accepting);
    closesocket(connecting);
    closesocket(listener);

    // ---- Names ----
    ADDRINFOA hints, *list = 0;
    zero(&hints, sizeof hints);
    hints.family = AF_INET;
    hints.socktype = SOCK_STREAM;
    check(getaddrinfo("127.0.0.1", "80", &hints, &list) == 0 && list && list->family == AF_INET && list->protocol == 6 && list->addrlen == 16 &&
          list->addr->port == htons(80) && list->addr->addr[0] == 127 && list->next == 0,
          "getaddrinfo: a numeric address and port");
    freeaddrinfo(list);
    hints.flags = 1;
    check(getaddrinfo(0, "8080", &hints, &list) == 0 && list->addr->addr[0] == 0 && list->addr->port == htons(8080), "getaddrinfo: passive, any address");
    freeaddrinfo(list);
    hints.flags = 0;
    check(getaddrinfo("localhost", "https", &hints, &list) == 0 && list->addr->addr[0] == 127 && list->addr->port == htons(443), "getaddrinfo: localhost, a service by name");
    freeaddrinfo(list);
    hints.flags = 4;
    check(getaddrinfo("example.com", 0, &hints, &list) == 11001, "getaddrinfo: AI_NUMERICHOST refuses a name");
    hints.flags = 0;
    char host[64];
    check(gethostname(host, sizeof host) == 0 && host[0] != 0, "gethostname");
    check(getaddrinfo(host, 0, &hints, &list) == 0 && list->addr->addr[0] == 10, "getaddrinfo: the machine's own name, its address");
    freeaddrinfo(list);
    check(inet_addr("1.2.3.4") == 0x04030201 && inet_addr("one") == 0xffffffff, "inet_addr");

    check(WSACleanup() == 0, "WSACleanup");
    ExitProcess(failures);
}
