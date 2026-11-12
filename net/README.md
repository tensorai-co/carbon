# net

A machine's network. Every program's sockets, in both kernels, go through `net`: it gives each machine a network of
its own (its addresses, its loopback, its DNS, the ports it publishes, the machines it can reach) and carries its
traffic over the host's own sockets. Like `display` and `audio`, it's a shared part with one folder per host, and
neither kernel writes networking of its own: lambda's socket system calls and atlas's Winsock are each a thin layer
over it.

**Phases 1 and 2 are built** (the shared part in memory, and out through each host): lambda's sockets work on it,
and `apt-get update` and `curl https://` work in `debian:13` on macOS, Linux and Windows. Ports in, policy, atlas's
Winsock and networks between machines are to do.

```sh
carbon run debian:13 -- curl https://example.com                      # out through the host, by default
carbon run --publish 8080:80 debian:13 -- python3 -m http.server 80   # in, from the host's port 8080 (to do)
carbon run --net none setup.exe                                       # loopback only (to do)
carbon run --net lab --name db postgres:17                            # machines on network "lab" reach it as "db" (to do)
```

```
 lambda                         atlas                                 another machine
   net/: socket syscalls,         net/: ws2_32 and mswsock (AFD),       on network "lab"
   epoll, netlink, resolv.conf    iphlpapi, dnsapi                            ▲
        │                              │                                      │
        ▼                              ▼                                      │
 net (shared, one per machine)                                                │
   the machine's interfaces and addresses     Unix sockets and loopback ──────┘ in memory, or the
   sockets: TCP, UDP, Unix, ICMP echo         machine to machine             host's loopback
   readiness and wakers                       published ports  ◄── the host's listeners
   policy: none, nat, host, rules             DNS: the machine's resolver
        │
        ▼
 the host's sockets, its reactor, its resolver
        │
        ▼
 the host's network
```

- **Sockets, not packets.** A kernel asks `net` for a socket, not for a network card. `net` keeps no TCP/IP stack: a
  TCP connection out of the machine is a host TCP connection, made when the program calls `connect`. There's one TCP
  stack between a program and the internet, the host's, so speed, window scaling, congestion control and path MTU are
  the host's own. This is what gVisor's host networking and WSL 1 do, and what rootless containers approximate with a
  user-mode stack.
- **A network of its own anyway.** Each machine still has its own addresses, its own loopback (two machines can both
  listen on `127.0.0.1:80`), its own ports, and a policy for what it may reach. Programs see an `eth0` with an
  address, a route and a DNS server, as they would in a container.
- **Nothing gets out another way.** A program's code can't open a host socket: exec's sandboxes deny networking to the
  stub process, and the vm backends have no network device. Every byte a machine sends passes through `net`, which is
  where its policy is.

`remote` is separate: it's Carbon talking to viewers, not a machine's programs, so it binds the host's addresses
itself and never goes through a machine's network.

## Principles

| Principle | What it means |
| --- | --- |
| **One stack** | The host's. `net` connects, listens and sends with host sockets, and adds no TCP/IP of its own |
| **One copy at most** | Data goes between guest memory and the host socket directly: exec's frames are host memory |
| **Non-blocking underneath** | `net` never blocks. It says what's ready and wakes a waker; blocking, timeouts, signals and EINTR, and completion ports are the kernel's, through `kernels-core::wait` |
| **Host-neutral** | One socket API and one set of errors for every host. Each kernel maps them to its ABI (errno, WSA codes, NTSTATUS); each host's quirks stay in its folder |
| **In memory where it can be** | Unix sockets, loopback and traffic between machines in one process never touch a host socket |
| **Deny by policy, in one place** | Every outbound connection, published port and DNS answer goes past the machine's policy, which can log it |
| **No dependencies** | `libc` and each host's system libraries. The DNS wire format is ours |

## What's built

- **`Network`** (`lib.rs`): one machine's. It's a `/16` on none of the host's addresses (the first of `10.89.0.0/16`,
  `10.90.0.0/16`, …, which avoids Docker's `172.17.0.0/16` and Podman's `10.88.0.0/16`): the machine is `.0.2`, and
  `.0.1` is its gateway and DNS server, both `net` itself. Programs see `lo` (`127.0.0.1/8`, `::1`) and `eth0` (MTU
  1500), and two routes: the network's own and the default through the gateway. Ports are the machine's, kept in one
  table for TCP and one for UDP, with `SO_REUSEADDR` and `SO_REUSEPORT`, and ephemeral ports from 32768.
- **`Socket`** (`socket.rs`): one state machine for every kind. A stream socket is idle, listening, joined to another
  in memory, or connected out through the host; a datagram socket keeps a queue that other sockets in the machine,
  the DNS server and the host's UDP sockets fill. It never waits: `ready` says what's ready, and `watch` registers a
  waker called whenever that may have changed. Blocking, timeouts and signals are the kernel's.
- **Inside the machine** (`stream.rs`): Unix stream and seqpacket sockets, and TCP over loopback (`127.0.0.0/8`, `::1`
  and the machine's own address), are two ends joined in memory, with TCP's semantics: listen backlogs, half-close,
  and a reset when an end closes with data unread. A Unix socket's name is its path (and the file the kernel made for
  it, so two spellings of one path meet) or an abstract name. What a kernel attaches to a message (`SCM_RIGHTS`'
  files) is opaque to `net` and goes with its first byte, and who sent it (`SO_PEERCRED`, `SCM_CREDENTIALS`) with
  each write. IPv6 sockets reach IPv4 ones through mapped addresses. The kernel can listen too: lambda's X11, Wayland
  and PulseAudio servers will take endpoints from `net` as programs do.
- **Out** (`sys/`): a TCP connection out is a host TCP connection, made when the program connects (on a thread, so
  `connect` starts at once); UDP has a host UDP socket behind it, opened at the first send. Programs only ever see the
  machine's address: in NAT terms, every machine is behind a full-cone NAT with the host's addresses. The host's
  sockets are std's, non-blocking, watched by one reactor thread for the process with `poll` (`WSAPoll` on Windows): it
  marks a socket ready once and stops watching that way until an operation finds it would block. What differs by host
  is `poll`, the host's addresses (`getifaddrs`, `GetIpAddrTable`) and its DNS servers (`/etc/resolv.conf`,
  `GetNetworkParams`).
- **DNS** (`dns.rs`): the machine's server, on the gateway's port 53 (UDP). The machine's own name and `localhost` it
  answers itself; addresses (A, AAAA) are looked up the host's way, so `/etc/hosts`, VPN split DNS and mDNS work as on
  the host; any other type goes to the host's DNS servers (or the machine's own, `Config::dns`) and its answer comes
  back as it was. Each question is answered on a thread of its own.
- **`Loop`** (`loop_host.rs`): a host with no sockets, an internet in memory with peers that echo, refuse, reset or
  stall, so `net` and the kernels test with no network.
- **Raw and packet sockets** are refused (`EPERM`), as in a rootless container.
- **One API for every host and kernel.** An `Addr` is IPv4, IPv6 or Unix. Errors are `net::Error` (`WouldBlock`,
  `InProgress`, `ConnectionRefused`, `ConnectionReset`, `AddrInUse`, `NetworkUnreachable`, `TimedOut`, `Denied` by
  policy, …), which lambda maps to errno and atlas will map to WSA codes and NTSTATUS. A socket's options are a typed
  `Options` (`set_options`), not a level and a number; each kernel maps its ABI's numbers onto it, and keeps the ones
  `net` doesn't act on, as Linux does. `Ancillary` is whatever a kernel attaches to a message, opaque to `net`.
  `Network::interfaces`, `routes` and `sockets` are what netlink, the interface ioctls and `/proc/net` show.

What each kernel does on it: lambda's socket system calls, sockets as files in `poll` and `epoll`, netlink, the
interface ioctls, `/proc/net/*` and its own `/etc/resolv.conf`, `/etc/hosts` and `/etc/hostname` are in the
[lambda's README](../kernels/lambda/README.md#how-it-works). atlas's Winsock is to do (below).

## The design still to build

### In: published ports

Nothing outside reaches a machine unless it publishes a port. `--publish 8080:80` has `net` listen on the host's port
8080 and hand each connection to whichever socket in the machine listens on port 80. Because the connection was never
re-made, the program's `accept` sees the real client's address and port, not the host's. With nothing listening in
the machine, the host side closes the connection at once, as a refused one would. UDP ports publish the same way
(`8080:80/udp`), with each datagram's real source.

### Modes and policy

| `--net` | What a machine has |
| --- | --- |
| `nat` (default) | Its own addresses and loopback; out through the host; in only through `--publish` |
| `none` | Loopback only. Every `connect` beyond it is refused, and DNS answers only the machine's own name |
| `host` | No network of its own: `bind` takes the host's ports and `connect` uses the host's addresses, as `docker run --network host`. The fastest and the least isolated, for a dev box |
| `NAME` | `nat`, plus the other machines on network `NAME`, which is made on first use |

Rules narrow what `nat` allows out, first match wins, after which the default allows:

```sh
carbon run --deny 0.0.0.0/0 --allow 10.0.0.0/8 --allow pypi.org:443 debian:13 -- pip install requests
```

A rule naming a host matches the addresses DNS gave the machine for it, so `pypi.org` keeps working as its addresses
change. Every refusal is logged (`carbon net log NAME`), so a program that fails to connect says why.

### The host

`host.carbon.internal` is the host's own loopback, for a program that needs a service running on the host. It's off
by default (a machine reaching the host's `localhost` reaches things that trusted it to be private), and `--allow
host` turns it on.

### Between machines

Machines on the same named network (`--net lab`) reach each other by address and by name. In one process, two
machines' sockets are joined in memory. Machines usually run in worker processes of their own (hvf allows one VM per
process), so a network also has a small registry in Carbon's state directory (`~/.carbon/net/lab/`): each machine
records its address and, for each port it listens on, a host endpoint of its own (a Unix socket on macOS and Linux, an
`AF_UNIX` socket or named pipe on Windows). Connecting to `10.89.0.3:5432` from another machine connects to that
endpoint, and the owning process hands the connection to its listener. There's no daemon: the registry is files, and
a machine's entries go when it stops. The machine's DNS server answers the names of the machines on its network.

### atlas

`kernels/atlas/src/net/`, with `os/ws2_32`, `os/mswsock`, `os/iphlpapi` and `os/dnsapi`:

- Winsock: `ws2_32`'s calls, overlapped I/O completing through I/O completion ports, `WSAPoll`, `select`,
  `WSAEventSelect`, `AcceptEx`, `ConnectEx`, `TransmitFile`; sockets as handles, which is how AFD makes them.
- `GetAdaptersAddresses`, `GetIpForwardTable`, `IcmpSendEcho`.
- `getaddrinfo`, `GetAddrInfoExW`, `DnsQuery_W` and `gethostbyname`, as host exports asking `net`'s resolver.
- Later, WinHTTP and WinINet with TLS (Schannel's part), checked against [`host`](../host/README.md)'s certificates.

### Each host, as planned

| | macOS (`net/macos`) | Linux (`net/linux`) | Windows (`net/windows`) |
| --- | --- | --- | --- |
| Sockets | BSD sockets | BSD sockets | Winsock, overlapped |
| Reactor | kqueue | epoll | an I/O completion port |
| ICMP echo | `SOCK_DGRAM` + `IPPROTO_ICMP` | the same, where `ping_group_range` allows; else refused | `IcmpSendEcho2` |
| Resolver | `DNSServiceQueryRecord` | the servers `/etc/resolv.conf` names | `DnsQueryEx` |
| Interfaces | `getifaddrs` | `getifaddrs` | `GetAdaptersAddresses` |
| Machine-to-machine endpoints | Unix sockets | Unix sockets | `AF_UNIX` sockets |

Today one `poll` reactor and std's sockets serve all three, with the few differences in `sys/`. The table is where
each grows when `poll` stops being enough. What `net` hides either way: `SO_REUSEADDR` (Windows' lets a second program
steal a port, so `net` uses `SO_EXCLUSIVEADDRUSE` there), each host's errors, Windows' readiness after a failed
`connect`, and macOS's `SIGPIPE` (`SO_NOSIGPIPE`).

### On the command line

| Option | Meaning |
| --- | --- |
| `--net nat\|none\|host\|NAME` | The machine's network; default `nat` |
| `--publish [IP:]HOST:MACHINE[/udp]`, `-p` | A host port that reaches a machine port; `IP` defaults to `0.0.0.0` |
| `--allow RULE`, `--deny RULE` | Outbound rules, first match wins: an address, a CIDR, a name or `host`, each with an optional `:PORT` |
| `--dns ADDR` | Where the machine's resolver sends what it doesn't know, instead of the host's resolver |
| `--hostname NAME` | What the machine calls itself; default its `--name` |

```sh
carbon net ls                       # NETWORK   SUBNET          MACHINES
carbon net inspect lab              # each machine's addresses and published ports
carbon net log web                  # connections and refusals, as they happen
carbon port web                     # 80/tcp -> 0.0.0.0:8080
```

## Layout

```
net/
  src/
    lib.rs        Network, Config, Addr, Error, ports
    socket.rs     Socket: the state machine every kind shares, readiness
    watch.rs      watchers and wakers
    stream.rs     stream and seqpacket sockets joined in memory: Unix, and TCP over loopback
    dns.rs        the machine's DNS server: the wire format, its own names, looking up and forwarding
    host.rs       the Host contract: what a host gives net
    sys/          the real host: std's sockets, the poll reactor; unix.rs and windows.rs for what differs
    loop_host.rs  Loop: an internet in memory, for tests
    tests.rs      net's tests at the Network level
  examples/net-conformance.rs       the same checks on every host
```

To come: `publish.rs` (the host's listeners), `policy.rs` (modes, rules, the log), `switch.rs` (named networks and
their registry). `net` depends on nothing of Carbon's; the kernels depend on it, and `machine` makes each machine's
`Network` and gives it to the kernel.

## Testing

```sh
cargo test -p net                                 # everything above on the Loop host, and the real host's sockets
cargo run -p net --example net-conformance        # the same checks on every host, through the host's own sockets
```

The conformance suite connects out to peers it starts on one of the host's own addresses (which a machine reaches the
way it reaches anywhere out): TCP connect, echo and half-close, a reset, a refused connection seen through
`SO_ERROR`, UDP both ways, an address the host's way, the machine's own name, and a TXT record from a DNS server of
the machine's own. All 17 checks pass on macOS, Linux and Windows (GCP VMs). lambda's `net.c` checks the same from
inside a machine.

As the phases below land, it adds: a published port reached from a host client with the client's address seen
inside, a policy refusal, and two networks in two processes. atlas gets a `winsock.c`, and real software is the test:
Debian's `curl`, `apt`, `ssh` and `python3 -m http.server`, and a Windows download over WinHTTP.

## To do

1. [x] **The shared part, in memory**, and lambda's sockets on it, in `epoll` and `poll` (`net.c`, both ISAs).
2. [x] **Out, on every host**: TCP and UDP out, the reactor, interfaces, the machine's DNS; lambda's netlink,
       `/proc/net` and `/etc` files. The conformance suite passes on all three hosts, and `curl https://` and
       `apt update` work in `debian:13`.
3. [ ] **In, and policy**: `--publish` (TCP and UDP, the client's real address seen inside), `--net none` and
       `host`, `--allow` and `--deny` with the log of refusals, `--dns`, `carbon port`. *Done means:* a web server in
       a machine answers the host's browser with the client's real address.
4. [ ] **atlas**: Winsock with overlapped I/O, `iphlpapi`, `dnsapi`; then WinHTTP and WinINet with TLS. *Done means:*
       `winsock.c` passes, and a Windows program downloads over HTTPS.
5. [ ] **Between machines**: named networks, the registry, names in DNS, `carbon net ls`, `inspect` and `log`.
       *Done means:* two machines in two worker processes talk by name.
6. [ ] **The rest, as programs ask**: ICMP echo (`ping` is refused for now: ICMP echo sockets get EACCES), IPv6 out
       with an address of the machine's own (a ULA `/64`, `fd89::/64`, used outward when the host has IPv6), broadcast
       and multicast (mDNS, SSDP) within a network, `SO_TIMESTAMP`, `TCP_INFO`.

- [ ] DNS over TCP (a reply is cut to fit 512 bytes instead).

## Open questions

1. **Packets, ever?** A virtual network card with a user-mode TCP/IP stack would allow raw sockets, VPN clients
   (`tun`), DHCP servers and protocols the host's sockets can't carry, at the cost of a second stack and a copy.
   *Recommendation:* not until a program people need requires it; sockets cover what rootless containers do.
2. **TLS for atlas.** WinHTTP and Schannel need TLS 1.2 and 1.3. *Recommendation:* the same answer as `remote`'s
   QUIC: `rustls` inside one atlas crate, the only one allowed it, until we write our own.
3. **The registry between processes, or a daemon?** Files and per-machine endpoints need nothing running, but leave
   stale entries when a worker dies. *Recommendation:* files, cleaned when a machine starts or `carbon net ls` finds
   an entry whose process is gone.
4. **The subnet.** *Recommendation:* `10.89.0.0/16` and up, as now, configurable in `~/.carbon/config`, skipping any
   subnet a host interface is on.

Settled: the name is `net` (short like `exec`, matching the kernels' own `net/` folders), and the host's `localhost`
is unreachable unless `--allow host`.
