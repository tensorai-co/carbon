// A Winsock server, for ARM64, x86-64 and x86: no C runtime. tests/programs/build.sh builds it. Unlike the other test
// programs it checks nothing itself: it's for reaching a Windows program's ports from outside the machine, through
// `--publish` or `--network host`.
//
//   serve.exe [PORT [CONNECTIONS]]      PORT 7 if not given; CONNECTIONS 0 (the default) serves until it's stopped
//
// On PORT over TCP, each connection is told who atlas says it's from ("atlas sees 192.168.1.5:50123"), then gets back
// the first thing it sends, and is closed. On PORT over UDP, each datagram comes back with "echo:" before it, to
// where it came from. It serves one at a time, waiting on both with select, and says each on its console.

typedef void *HANDLE;
typedef unsigned int DWORD;
typedef unsigned char BYTE;
typedef __UINTPTR_TYPE__ SOCKET;
typedef struct { unsigned short family, port; BYTE addr[4]; BYTE zero[8]; } SOCKADDR_IN;
typedef struct { unsigned count; SOCKET fds[64]; } FD_SET;

#define K __declspec(dllimport)
K HANDLE __stdcall GetStdHandle(DWORD);
K int __stdcall WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
K void __stdcall ExitProcess(unsigned);
K char *__stdcall GetCommandLineA(void);
K int __stdcall WSAStartup(unsigned short, void *);
K int __stdcall WSAGetLastError(void);
K SOCKET __stdcall socket(int, int, int);
K int __stdcall bind(SOCKET, const void *, int);
K int __stdcall listen(SOCKET, int);
K SOCKET __stdcall accept(SOCKET, void *, int *);
K int __stdcall send(SOCKET, const char *, int, int);
K int __stdcall recv(SOCKET, char *, int, int);
K int __stdcall sendto(SOCKET, const char *, int, int, const void *, int);
K int __stdcall recvfrom(SOCKET, char *, int, int, void *, int *);
K int __stdcall closesocket(SOCKET);
K int __stdcall select(int, FD_SET *, FD_SET *, FD_SET *, const void *);
K unsigned short __stdcall htons(unsigned short);

#define INVALID ((SOCKET)~(__UINTPTR_TYPE__)0)
#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2

static HANDLE out;
static unsigned len(const char *s) { unsigned n = 0; while (s[n]) n++; return n; }
static void say_n(const char *s, unsigned n) { DWORD w; WriteFile(out, s, n, &w, 0); }
static void say(const char *s) { say_n(s, len(s)); }

/// Appends `n` in decimal at `at`; where it ends.
static char *decimal(char *at, unsigned n) {
    char digits[12];
    int i = 0;
    do { digits[i++] = (char)('0' + n % 10); n /= 10; } while (n);
    while (i) *at++ = digits[--i];
    return at;
}

/// "a.b.c.d:port" at `at`; where it ends.
static char *address(char *at, const SOCKADDR_IN *a) {
    for (int i = 0; i < 4; i++) {
        at = decimal(at, a->addr[i]);
        *at++ = i < 3 ? '.' : ':';
    }
    return decimal(at, htons(a->port));
}

/// The command line's numbers after the program's name, as many as `count`; 0 for one not there.
static void numbers(unsigned *got, int count) {
    const char *s = GetCommandLineA();
    // The program's name, quoted or not.
    if (*s == '"') { s++; while (*s && *s != '"') s++; if (*s) s++; }
    else while (*s && *s != ' ') s++;
    for (int i = 0; i < count; i++) {
        got[i] = 0;
        while (*s == ' ') s++;
        while (*s >= '0' && *s <= '9') got[i] = got[i] * 10 + (unsigned)(*s++ - '0');
    }
}

static BYTE wsa[408];
static char buf[512], line[640];
// fd_sets are big: static, so nothing calls memset to clear them.
static FD_SET reads;

void mainCRTStartup(void) {
    out = GetStdHandle((DWORD)-11);
    unsigned args[2];
    numbers(args, 2);
    unsigned port = args[0] ? args[0] : 7, connections = args[1];
    if (WSAStartup(0x202, wsa) != 0) { say("WSAStartup failed\n"); ExitProcess(1); }
    SOCKADDR_IN any = { AF_INET, htons((unsigned short)port), {0, 0, 0, 0}, {0} };
    SOCKET tcp = socket(AF_INET, SOCK_STREAM, 0), udp = socket(AF_INET, SOCK_DGRAM, 0);
    if (bind(tcp, &any, sizeof any) != 0 || listen(tcp, 16) != 0 || bind(udp, &any, sizeof any) != 0) {
        char *end = decimal(line + 0, (unsigned)WSAGetLastError());
        say("can't take the port: error ");
        say_n(line, (unsigned)(end - line));
        say("\n");
        ExitProcess(2);
    }
    char *end = decimal(line, port);
    say("serving TCP and UDP on port ");
    say_n(line, (unsigned)(end - line));
    say("\n");
    unsigned served = 0;
    while (!connections || served < connections) {
        reads.count = 2;
        reads.fds[0] = tcp;
        reads.fds[1] = udp;
        if (select(0, &reads, 0, 0, 0) <= 0) { say("select failed\n"); ExitProcess(3); }
        for (unsigned i = 0; i < reads.count; i++) {
            SOCKADDR_IN from;
            int n = sizeof from;
            if (reads.fds[i] == tcp) {
                SOCKET c = accept(tcp, &from, &n);
                if (c == INVALID) continue;
                char *at = line;
                for (const char *p = "atlas sees "; *p; p++) *at++ = *p;
                at = address(at, &from);
                *at++ = '\n';
                send(c, line, (int)(at - line), 0);
                say("tcp: ");
                say_n(line + 11, (unsigned)(at - line - 11));
                int got = recv(c, buf, sizeof buf, 0);
                if (got > 0) send(c, buf, got, 0);
                closesocket(c);
                served++;
            } else if (reads.fds[i] == udp) {
                int got = recvfrom(udp, buf, sizeof buf - 5, 0, &from, &n);
                if (got < 0) continue;
                char *at = line;
                for (const char *p = "echo:"; *p; p++) *at++ = *p;
                for (int k = 0; k < got; k++) *at++ = buf[k];
                sendto(udp, line, (int)(at - line), 0, &from, sizeof from);
                char *e = address(line, &from);
                *e++ = '\n';
                say("udp: ");
                say_n(line, (unsigned)(e - line));
            }
        }
    }
    say("served\n");
    ExitProcess(0);
}
