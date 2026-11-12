# lambda

lambda runs Linux programs. The system call is the line between a program and its kernel, and the image brings
everything above it (glibc or musl, the dynamic loader, bash, apt); lambda brings the rest of what a Linux kernel
provides, all of it host Rust: processes, memory, files, sockets, signals, `/dev`, `/proc`, `/sys`, a vDSO, and a
Wayland compositor and an X server on Carbon's desktop.

**Where it stands.** Debian 13 runs, ARM64 natively and x86-64 translated or native: bash with job control, ptys,
threads, `epoll`, sockets, and `apt-get install` of curl, Python and git, which then work over HTTPS. GTK 3 and 4,
Qt 6, foot, xterm and weston's clients draw on the desktop. Of Linux 7.4's system calls, lambda decodes 318 of 326 on
ARM64 and 361 of 369 on x86-64.

## Layout

```
kernels/lambda/src/
  lib.rs          boot, the dispatcher
  abi.rs          Linux's structs and constants (private: nothing in the program needs them)
  sys/            each ISA's numbers mapped to one enum at the edge: arm64.rs, x86_64.rs (i386.rs to come)
  process/        clone, execve, loading ELF, credentials
  signal.rs       delivery, sigreturn, each ISA's signal frame
  mm/             mmap, mprotect, brk, mremap, madvise; shared memory, memfds and SysV segments; mlock, mseal, NUMA
  fs/xattr.rs     extended attributes, the twelve calls and the four *xattrat forms
  security/       capget and capset, seccomp's filters, LSM and Landlock answers
  admin.rs        host and domain names, reboot, the kernel log, quotas, what a machine may not do
  ipc/            SysV message queues and semaphores, POSIX message queues
  ns.rs           mount, UTS and IPC namespaces; unshare, setns, listns, /proc/<pid>/ns
  fs/             the file table over layers: pipes, poll, epoll, eventfd, timerfd, metadata, inotify, AIO; mounts
                  and the path walk (mount.rs)
  net/            the socket syscalls over net, netlink, /proc/net
  time.rs tty.rs  clocks and timers; the line discipline, the console and ptys
  os/             what Linux itself provides, made in memory: /dev, /proc; wayland/, the Wayland compositor
                  answering $XDG_RUNTIME_DIR/wayland-0 on the desktop; x11/, the X server (and its window
                  manager's part) answering /tmp/.X11-unix/X0 on the same desktop
                  vdso.rs, the vDSO and its clock page; sysfs.rs, /sys made each time it's read
                  to come: pulse/ (a PulseAudio server)
```

Each ISA's system call numbers become one `Sys` at the edge (`sys/`), with x86-64's old calls (`open`, `stat`, `fork`)
turned into their modern forms and `clone`'s arguments put in one order, so nothing past it knows the ISA. i386 will
fix its own quirks (`mmap2`) there the same way.

## Running and testing

```sh
cargo build --example run
codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run   # macOS
target/debug/examples/run path/to/static-binary args...    # ARM64 natively on Apple silicon; x86-64 translated
```

The runner uses the host's `vm` backend; `CARBON_ISOLATION=sandbox` (or `auto`) picks the sandbox. On Linux, kvm needs
the user in the `kvm` group. `CARBON_TRACE=1` prints every system call with its result, every seccomp verdict but
allow and errno, and each thread that gives its CPU up, waits for one, or takes one again.

The freestanding test programs in `tests/programs/` build with Homebrew's LLVM, all like `hello.c`:

```sh
clang --target=aarch64-linux-gnu -nostdlib -static -ffreestanding -fno-stack-protector -fno-builtin \
    -fuse-ld=lld -O2 kernels/lambda/tests/programs/hello.c -o hello-aarch64
clang --target=x86_64-linux-gnu -nostdlib -static -ffreestanding -fno-stack-protector -fno-builtin \
    -fuse-ld=lld -O2 kernels/lambda/tests/programs/hello.c -o hello-x86_64
```

On this Mac, through hvf, all of them pass. Each program's opening comment says what it checks.

| Program | What it covers |
| --- | --- |
| lambda, ARM64 native and x86-64 translated | The basics: arguments, memory, files, clocks, pipes, fork and wait |
| lambda's `signals.c`, both ISAs | Handlers and `rt_sigreturn`, blocking and pending, job control and ptys |
| lambda's `vdso.c`, both ISAs | The vDSO found and used as a C library does, its clocks checked against the system call |
| lambda's `threads.c`, both ISAs | Threads by `clone` sharing memory on several CPUs, futexes, TLS, and joining |
| lambda's `code.c`, both ISAs | Code written at run time, as JITs do: rewritten, called into, and W^X |
| lambda's `events.c`, both ISAs | eventfd, timerfd, epoll, signalfd, and `ppoll`'s signal mask |
| lambda's `net.c`, both ISAs | Unix stream, datagram and seqpacket sockets, descriptors passed, TCP and UDP on loopback, netlink, `/proc/net` and DNS |
| lambda's `files.c`, both ISAs | Hard links, permissions, owners and times, a file removed while open, locks, `close_range` |
| lambda's `io.c`, both ISAs | Vectored and positioned I/O, `sendfile`, `splice` and the rest, truncating and allocating, syncing |
| lambda's `procs.c`, both ISAs | Resource limits, nice values and scheduling, CPU affinity and times, `waitid`, robust futexes, `execveat` |
| lambda's `sigtimers.c`, both ISAs | Real-time signals, `rt_sigtimedwait`, interval and POSIX timers, setting the clocks, pidfds |
| lambda's `memory.c`, both ISAs | Shared and private mappings across a fork, memfd and its seals, SysV shared memory, `msync`, `mlock` |
| lambda's `x11.c`, both ISAs, with `CARBON_DISPLAY` | An X11 client with no Xlib: windows, properties, images, the extensions and their requests |
| lambda's `wayland.c`, both ISAs, with `CARBON_DISPLAY` | A Wayland client with no libwayland: the globals, input, shared memory, toplevels and popups |
| lambda's `ipc.c`, both ISAs | SysV message queues and semaphores, POSIX message queues, and futex2 |
| lambda's `watch.c`, both ISAs | inotify, and Linux AIO |
| lambda's `security.c`, both ISAs | Capabilities, securebits, `prctl` and seccomp |
| lambda's `sysfs.c`, both ISAs | sysfs's CPUs, classes and devices, and `/proc`'s names |
| lambda's `mounts.c`, both ISAs | Mount tables, tmpfs and bind mounts, propagation, `chroot` and `pivot_root` |
| lambda's `ns.c`, both ISAs | Namespaces, and the new mount API |
| lambda's `attrs.c`, both ISAs | Extended attributes, `mknod`, and `openat2` |
| lambda's `ptrace.c`, both ISAs | `ptrace`'s stops, memory and system calls, and `process_vm_readv`/`writev` |
| lambda's `marks.c`, both ISAs | fanotify, `statmount`/`listmount`, `remap_file_pages` and x86-64's old calls |

The x86-64 programs are built as they come, SSE and SSE2 included. Two of lambda's check the translator bit for bit,
each generated by a Python script beside it (`python3 sse.py > sse.c`), x86-64 only:

- `sse.c`: 186 SSE and SSE2 cases, against `sse.py`'s own model of x86.
- `x87.c`: 105 x87 cases (loads and stores in every format, the arithmetic in every form, compares, `fcmov`, `fist`
  with each rounding mode, `fprem` loops, `fxam`, the control word and environment, `sahf`/`lahf`). The answers are
  what a real x87 stores, and keep to results its 64-bit precision and exec's 53-bit one agree on. The same file
  builds as an x86-64 macOS program (`clang -arch x86_64 x87.c`), and all but `ffreep` pass on Rosetta 2's x87.

And one written by hand, `x86.c` (x86-64 only): 28 checks of general-purpose instructions real programs needed: 8- and
16-bit rotates (with CF, by an immediate and by `cl`), `mul`, `imul` and `div`/`idiv`; `stc`, `clc`, `cmc`; `lock
bts`/`btr`/`btc`; bit tests on memory whose bit number reaches other words; and `fxsave`/`fxrstor`, the image's
layout and the registers back. The instruction runs in inline assembly on values the compiler can't see, and its
answer is the compiler's for the same constants, so it passes on real x86 too.

**On Linux and Windows hosts** (x86-64, GCP VMs with nested virtualization), through kvm and winkvm, these pass on
x86-64: hello (12), signals (11), `sse`, `x87`, `threads` (9, its 300 threads and 300 processes included), `code`,
`events` (36) and `net` (36); there `sse` and `x87` run on the real CPU, which confirms their answers. Through the
Linux sandbox they pass too, except the fork and wait checks, which need a second CPU. lambda doesn't run in the
Windows sandbox yet, since its programs' `syscall` needs rewriting into calls to that sandbox's gate.

## Debian

lambda boots Debian 13 (ARM64) from a tar of its root file system, and runs its own `bash` through Debian's
`ld-linux` and glibc:

```sh
# Any tar of an ARM64 root file system works, for instance from Docker:
docker export $(docker create --platform linux/arm64 debian:13) > debian.tar

target/debug/examples/run --root debian.tar /bin/bash -c 'uname -a; ls / | head -3'
target/debug/examples/run --root debian.tar /bin/bash -i      # interactive, without job control
```

Loading the tar's 5,400 files takes about half a second in a debug build, and 40 ms in a release one. What's been seen
working: `bash -c`, scripts on stdin, and `bash -i`; `ls`, `find`, `grep`, `sed`, `sort`, `tr`, `paste`, `head`, `wc`,
`id`, `uname`, `sleep`, `dpkg -l` and `apt --version`; pipelines, command substitution, here-strings, files written
and read back in `/tmp`, exit statuses, and `yes | head` (SIGPIPE).

When stdin and stdout are a terminal, it becomes the machine's **console**: the host terminal goes into raw mode, and
lambda's own line discipline edits lines, echoes, and turns ^C, ^\ and ^Z into signals for the foreground process
group. So bash gets job control and readline: ^C (status 130), ^Z and `fg`, `jobs`, `kill %1`, background jobs with
`&`, arrow keys and history. Resizing the host's terminal sends SIGWINCH. Inside the machine, `/dev/ptmx` makes pty
pairs: util-linux's `script` runs a command in one.

Use a release build for interactive use: a debug build's fork and exec take about 250 ms, long enough for a quick ^Z to
catch a job before it has run.

**The network** is `net`'s: `carbon run debian:13 -- apt-get update` fetches and verifies Debian's indexes (10 MB at
about 1.8 MB/s on the Mac in a debug build; 2.5 MB/s and 1.7 MB/s for x86-64 on the Linux and Windows VMs, through kvm
and winkvm), and `curl https://` works on all three, over HTTP/2, with Debian's own CA bundle:
`www.debian.org`, Wikipedia and PyPI answer 200, and an expired certificate is refused. `getent hosts` and curl find
names through the machine's DNS server.

**`apt-get install`** works in ARM64 `debian:13` on the Mac: `apt-get install curl` (30 packages: unpacked, their
maintainer scripts run, `update-ca-certificates` building the CA store), then `python3` and `git` (36 more, Python
byte-compiling as it goes); `dpkg -l` shows all 147 installed, and the installed curl, Python's `urllib` and
`git clone` work over HTTPS. A debug build installs curl in about 45 seconds.

**Debian 13 for x86-64** boots the same way, translated on Apple silicon (and natively through kvm and winkvm):

```sh
docker export $(docker create --platform linux/amd64 debian:13) > debian-amd64.tar
target/release/examples/run --root debian-amd64.tar /bin/bash -c 'seq 100000 | sha256sum'
```

Seen working: bash, `ls`, `cat`, `sort`, `grep`, `wc`, `sha256sum`, and with x87, `printf "%f"`, `seq`, `awk`,
`numfmt` and `sort -g`. With blocks chained, `seq 100000` takes 0.06 s and a 100,000-round bash loop 1.5 s on the Mac.
`apt-get update` and `apt-get install curl`, then `python3` and `git`, work translated as they do on ARM64 (curl's
install takes about 4 minutes in a debug build, against 45 seconds natively), and so do the installed curl, Python
(`hashlib`, `zlib`, `urllib` over HTTPS, `subprocess`) and `git clone` over HTTPS, which `git fsck` finds sound.
libm's `long double` functions don't work yet (the translator has no x87 transcendentals).

## How it works

For lambda, the syscall is the line between program and kernel, and the image brings glibc or musl, the
dynamic loader and everything above them. What lambda brings is what a Linux kernel provides beyond syscalls, all of it
host Rust.

The root is the image's layers (Debian's, say) plus `os/`'s file systems. lambda loads the
  command, its ELF interpreter from the image, and its vDSO (with the code signal handlers return through) from
  memory. A static
  program needs no image: `run ./static-binary` runs on an empty root plus `os/`.

- **Processes** (`process/`): `execve` with `#!` scripts, ELF loading with an interpreter and position-independent
  programs, the first stack with its auxiliary vector; `clone` for threads and processes (fork is exec's
  `fork_space`; a new thread joins its process before it can run, so one that ends at once is never left listed);
  `wait4` with `WUNTRACED` and `WCONTINUED`; exit and orphans; futexes; sessions and process groups; credentials
  (`creds.rs`: real, effective, saved and filesystem IDs and supplementary groups, with Linux's rules, so a program
  that drops root, as `apt` does to `_apt`, can't get it back; files' permissions aren't checked against them yet), and
  capabilities: effective, permitted, inheritable, bounding and ambient sets and securebits, changed as Linux changes
  them when user IDs change and at `execve`; changing IDs asks `CAP_SETUID` and `CAP_SETGID`.
- **Sockets** (`net/`): the socket system calls on `net` (`socket`, `socketpair`, `bind`, `listen`, `accept4`,
  `connect`, `send*`, `recv*` including `sendmsg`, `recvmsg`, `sendmmsg` and `recvmmsg`, `shutdown`, `getsockopt`
  and `setsockopt`), with Linux's `sockaddr` layouts for Unix, IPv4, IPv6 and netlink. A socket is a file, so `read`,
  `write`, `poll`, `select` and `epoll` work on it; it blocks as its file and flags say, with `SO_RCVTIMEO` and
  `SO_SNDTIMEO`, and EPIPE sends SIGPIPE unless `MSG_NOSIGNAL`. `SCM_RIGHTS` passes files; `SO_PEERCRED` and
  `SCM_CREDENTIALS` say who's at the other end. A Unix socket bound to a path makes a socket file there. Options
  `net` doesn't act on are kept and given back, as Linux does. The interface ioctls (`SIOCGIFCONF`, `SIOCGIFFLAGS`,
  `SIOCGIFADDR`, …) and netlink's routing family (`RTM_GETLINK`, `RTM_GETADDR`, `RTM_GETROUTE`) describe `lo` and
  `eth0`; `/proc/net/{tcp,tcp6,udp,udp6,unix,dev,route,if_inet6}` list the machine's sockets; `/etc/resolv.conf`,
  `/etc/hosts` and `/etc/hostname` are the machine's own, made in memory. Raw and packet sockets are EPERM, and ICMP
  echo sockets EACCES, so `ping` says it can't.
- **Signals** (`signal.rs`): per-thread masks and pending sets, process-wide pending signals with their siginfo,
  default actions (terminate, ignore, stop, continue), handlers with frames on the program's stack for each ISA,
  `rt_sigreturn` through the vDSO's code when there's no `SA_RESTORER`, `SA_RESTART`, `SA_ONSTACK` with
  `sigaltstack`, `SA_NODEFER`, `SA_RESETHAND`, `rt_sigsuspend`, `rt_sigpending`, `signalfd` (woken when a signal it
  reads becomes pending), the signal mask of `ppoll`, `pselect6` and `epoll_pwait`, SIGCHLD with its code and
  status, SIGPIPE, and faults as SIGSEGV, SIGILL, SIGBUS, SIGFPE and SIGTRAP.
- **Terminals** (`tty.rs`): one line discipline (canonical and raw, `VMIN`/`VTIME`, erase, word-erase, kill, EOF,
  echo with `ECHOCTL`, `ONLCR`, signal keys) for the console and for pty pairs; `TCGETS`/`TCSETS*`, window sizes,
  foreground groups, controlling terminals, `TIOCGPTN`, `TIOCSPTLCK`, `TIOCGPTPEER`; SIGTTIN for background reads;
  SIGHUP when a session leader leaves.
- **Memory** (`mm/`): `mmap` (anonymous and file-backed through the page cache), `munmap`, `mprotect`, `mremap`,
  `madvise`, `brk`. Mapping only makes a region; pages are filled when the program faults on them. Shared memory
  (`mm/shared.rs`) is frames lambda owns, made on first touch, that every mapping of it maps: `MAP_SHARED`
  anonymous memory and `/dev/zero`, memfds (with seals) and SysV segments, shared across fork. `mm/control.rs` has
  `msync`, `mincore`, the `mlock` family, `mseal`, NUMA on one node, and `process_madvise`.
- **IPC** (`ipc/`): SysV message queues and semaphores, one table each for the machine, with `SEM_UNDO` undone when
  a process ends; POSIX message queues by name, each open as a file that `poll` and `epoll` watch and `read` gives
  the status of, with `mq_notify`'s signal. A process-shared futex in shared memory (and futex2's `futex_waitv`, on
  `kernels_core::wait`) is keyed by its place in that memory, so processes meet on it.
- **Files** (`fs/`): a file table per process shared by `dup` and fork, files and directories over `layers`, pipes
  that block, `/dev/null`, `zero`, `random` and `urandom`, `statfs`, `FIONREAD` on pipes. An open file keeps its
  inode (`layers`' `hold`), so one removed while open is read and written until it's closed, as programs' nameless
  temporary files are; `O_PATH` opens a symbolic link itself. Hard links (`linkat`), `chmod`, `chown` and
  `utimensat` (`fs/meta.rs`), and `close_range`. `flock` and `fcntl`'s record locks are granted at once, and
  `F_GETLK` says nothing's in the way: no process's lock is checked against another's yet.
- **Waiting on files** (`fs/poll.rs`, `fs/epoll.rs`, `fs/events.rs`): `poll`/`ppoll`/`select`/`pselect6`, `epoll`
  (level- and edge-triggered, one-shot, an epoll inside another, `epoll_pwait2`), `eventfd` (with `EFD_SEMAPHORE`)
  and `timerfd` on the wall or monotonic clock. Nothing is checked on a timer: each file that can become ready (pipes,
  ttys, host stdin, signalfds, eventfds, timerfds, epolls) keeps `Watchers` (`kernels_core::wait`), a thread waiting
  on files is among the watchers of each while it waits, and an epoll's interest in a file is one of that file's
  watchers for as long as it's there. A timer has no thread: a waiter sleeps until its next expiry at the latest, and
  whoever first sees it expired tells its watchers.
- **Mounts and roots** (`fs/mount.rs`): every path is walked by lambda, component by component: a covered name
  leads into what's mounted on it, `..` at a mount's root leads out to where it's mounted, and neither `..` nor an
  absolute link climbs above the process's root (`chroot`, `fchroot`). `mount` binds (with `MS_REC`), moves,
  remounts (a read-only mount's writes are EROFS), sets propagation, and mounts tmpfs (a volatile tree of `layers`',
  never logged or saved, sized by `size=` for `statfs`), proc, devpts and mqueue; `umount2`, lazily too;
  `pivot_root`. `/proc/<pid>/mounts`, `mountinfo` and `/proc/mounts` list them; `statfs` gives each mount's type and
  flags. The table is a mount namespace's, held by each process: there's one today.
- **Extended attributes** (`fs/xattr.rs`): the twelve `*xattr` calls and the four `*xattrat` forms, over attributes
  `layers` keeps with each file. Linux's rules: the `user.`, `trusted.`, `security.` and `system.` namespaces, who may
  read or change each (`user.` only on files and directories, `trusted.` for `CAP_SYS_ADMIN`), `XATTR_CREATE` and
  `XATTR_REPLACE`, and the sizing of `get` and `list`.
- **Special files** (`fs/mod.rs`, `fs/pipe.rs`): `mknod`/`mknodat` make device nodes (with `CAP_MKNOD`) and FIFOs; a
  FIFO's readers and writers meet on one shared buffer, a reader waiting for a writer and the other way, as Unix does.
- **`openat2`**: `open_how` taken strictly (unknown flags and resolve bits refused), with `RESOLVE_NO_SYMLINKS`,
  `RESOLVE_BENEATH`, `RESOLVE_IN_ROOT` and `RESOLVE_NO_XDEV` enforced in the path walk.
- **Namespaces** (`ns.rs`): each process holds its mount, UTS (host and domain names) and IPC (System V objects and
  POSIX message queues) namespace, and shares the machine's one PID, network, user, cgroup and time namespace.
  `clone`'s `CLONE_NEWNS`/`NEWUTS`/`NEWIPC`, `unshare` and `setns` give it its own of the first three; the others'
  `CLONE_NEW*` is EINVAL, since lambda keeps one of each. Each namespace has an inode of its own, so
  `/proc/<pid>/ns/<kind>` reads as `<kind>:[inode]`, opens to an fd `setns` takes, and `listns` enumerates them.
- **The new mount API** (`fs/mount.rs`): `fsopen` and `fsconfig` build a file system (tmpfs, proc, devpts, mqueue),
  `fsmount` makes it a detached mount, and `move_mount` attaches it; `open_tree` clones a subtree detached; `fspick`
  and `mount_setattr` change an existing mount (read-only and the rest). All on the same table as `mount`.
- **Watching files** (`fs/inotify.rs`): inotify instances as files that `read`, `poll` and `epoll` know. `layers`
  tells an observer, set at boot, of every change to the disk (made, linked, removed, moved, written, metadata set),
  whatever system call made it; opens, reads and closes are told from `fs`. Each event goes to the watches on the file
  and on a directory it's in, with its name; for a write or a chmod, which name no directory, the watched directories
  are looked through for it. Consecutive identical events merge, as on Linux.
- **Linux AIO** (`fs/aio.rs`): each request is done as it's submitted, as Linux does a buffered file's, and its
  completion waits for `io_getevents`; an eventfd can be told. A context's ID is memory mapped for it, zeroed, so
  libaio's ring fast path sees no ring and asks the kernel.
- **Security** (`security/`): `capget` and `capset`; `prctl`'s bounding and ambient sets, securebits, keep-caps and
  `no_new_privs`. seccomp: strict mode, and classic-BPF filters checked as Linux checks them and run before each
  system call on its `seccomp_data` (the number as the program's ISA has it, the audit arch, the address, the
  arguments); the most restrictive answer of all of them wins: an errno, SIGSYS with its siginfo, or the thread or
  process killed. The only Linux Security Module is `capability`; Landlock answers as a kernel booted with it off.
- **The machine** (`admin.rs`): its host and domain names (`uname`, `/proc/sys/kernel`); `reboot` ends it as it
  ends a container, halt and power-off as SIGINT to its first process, restart as SIGHUP; `syslog` reads the kernel's
  log, as `dmesg` does; no quotas; kernel modules, kexec, process accounting, BPF, performance counters and x86's I/O
  ports refused, as in a container.
- **fanotify** (`fs/fanotify.rs`): classic notification groups, with marks on files, a directory's children or a
  whole file system (a mount's mark covers its file system), ignore masks and flushing. Open, access, modify and close
  events come from where inotify's do, each with the process (or thread) that caused it, merged per file and process;
  a read opens a descriptor on each event's file in the reader, which makes no events itself.
- **Mount IDs** (`fs/mount.rs`): `statmount` and `listmount` on the same table as `mount`, with Linux's 64-bit IDs.
- **Tracing** (`process/ptrace.rs`, `process/vm.rs`): `ptrace`, per thread, by a tracer that needn't be the parent
  (`TRACEME`, `ATTACH`, `SEIZE`). A traced thread stops where Linux's would: a signal about to be delivered (which
  the tracer may let through, change or drop), each system call's entry and exit, a group stop, `PTRACE_INTERRUPT`,
  and the fork, vfork, clone, exec and exit events, with new threads and processes traced from their first
  instruction (`Guest::start`). The tracer hears of each stop, and of its tracees' ends, through `wait4`/`waitid` as
  of a child's. A thread's registers are only its own CPU's, so a stopped thread leaves a copy for its tracer and
  takes back what was changed (orig_rax or `NT_ARM_SYSTEM_CALL` to change or skip a call). Another process's memory
  is read and written in turns in its space (`Cx::elsewhere`), its pages filled as its touch would; a poke to code it
  can't write goes to a private copy of the page. `process_vm_readv`/`writev` use the same way in.
- **The OS** (`os/`): `/dev` put on the disk at boot, and `/proc/self/{exe,cwd,fd,maps,status}`, `cpuinfo`,
  `meminfo`, `uptime` made as they're read.

## To do

### System calls

Against Linux 7.4 (`scripts/syscall.tbl` for ARM64, `syscall_64.tbl` for x86-64). A call lambda doesn't decode
gets ENOSYS.

- **Not decoded:** `io_uring_setup`, `io_uring_enter` and `io_uring_register` (a ring in shared memory, worked on
  enter); `userfaultfd`; the keyrings (`add_key`, `request_key`, `keyctl`); `rseq_slice_yield`, which needs `rseq`.
- **Decoded, but stubs:** `clone3` and `rseq` are ENOSYS (glibc falls back; `rseq` is registered by every thread
  from glibc 2.35); `membarrier` answers 0 to everything, `MEMBARRIER_CMD_QUERY` included; `prctl`'s options past
  security and names answer 0 (`PR_SET_PDEATHSIG` and `PR_SET_CHILD_SUBREAPER` need real ones); `fadvise64` is
  accepted and ignored, which is fine.

To check again, fetch the two tables and compare them with the match arms in `src/sys/arm64.rs` and `x86_64.rs`:

```sh
curl -sLO https://raw.githubusercontent.com/torvalds/linux/master/scripts/syscall.tbl
curl -sLO https://raw.githubusercontent.com/torvalds/linux/master/arch/x86/entry/syscalls/syscall_64.tbl
```

### The kernel

- **Processes:** `execve` with other threads running (end them first); a `vfork` that shares memory; process groups
  and sessions as job control needs them; SIGTTOU, the FP state in the signal frame itself, and core dumps;
  x86-64's `MAP_32BIT`; i386 programs (`sys/i386.rs`, with exec's 32-bit translation).
- **Memory:** `MAP_SHARED` of a file on the disk written back and kept in step with `read` and `write` (anonymous
  shared memory, memfds and SysV segments are shared already).
- **Files:** permissions checked against credentials (the credentials are done); locks that hold between processes
  (`flock`, record and OFD locks); `EPOLLEXCLUSIVE` and `EPOLLWAKEUP` (accepted, ignored); inotify's
  `IN_DELETE_SELF` at an open file's last close rather than the unlink, and `IN_EXCL_UNLINK`; AIO's
  `IOCB_CMD_POLL`; fanotify's FID modes, pidfds and permission events; extended attributes in a frozen image or
  from a tar; `openat2`'s `RESOLVE_NO_MAGICLINKS` checked, not implied.
- **Mounts and namespaces:** `/proc/<pid>/ns` listable (each link reads, opens and `setns` already); separate PID,
  network, user, cgroup and time namespaces (lambda keeps one of each); `open_tree_attr`'s attributes; `nosuid`,
  `nodev` and `noexec` enforced, not only shown; read-only decided by the mount a file is on, not its path;
  unmounting checking for open files; cgroups and overlay mounts (ENODEV today).
- **Security:** seccomp filters per thread, and user notification; file capabilities and setuid bits at `execve`;
  capabilities, not effective root, asked for IPC, limits and nice; Landlock enforced.
- **IPC:** SysV objects' permission bits; `mq_notify`'s `SIGEV_THREAD`, and a registration dropped when its
  descriptor closes; futex2's masks, sizes other than 32 bits, and `FUTEX2_NUMA`.
- **Tracing:** single-stepping (exec has no trap after one instruction, so gdb can't step over a breakpoint),
  hardware breakpoints, `PTRACE_O_TRACESECCOMP`'s stops, `PTRACE_EVENT_VFORK_DONE`, `PTRACE_PEEKSIGINFO`; a traced
  thread's group stop is its own, not its process's. gdb attaching to a multi-threaded process hangs.
- **Scheduling:** nice, policies, affinity and I/O priority are kept per process, not per thread, change nothing on
  the host, and aren't in `/proc/<pid>/stat`; time slices, so threads that spin rather than wait give a vCPU up (a
  thread blocked in a wait already does); more than one CPU in the process sandbox.
- **The vDSO:** its counter on hosts other than Apple silicon (it makes system calls there: on KVM the guest's TSC
  needs calibrating); `time`, `getcpu` and `getrandom` with no system call; unwinding information for its signal
  return.
- **sysfs and `/proc`:** a DRM render node's and input devices' entries, with those devices; block devices; cgroups;
  `uevent` over netlink; `/proc/<pid>` and `/proc/net` as listable directories; the rest of `/proc`.
- **Other hosts:** `apt install` natively on Linux and Windows hosts (it works on the Mac, in both images); a
  console on Windows hosts, tried with a full-screen editor.
- **Sound:** a PulseAudio server, `os/pulse`, on `audio`.

### The desktop and graphics

lambda's display servers put windows on `display`'s desktop; the desktop's own to-do is in
[`display/README.md`](../../display/README.md#to-do). The rules: anything two of win32k, Wayland and X11 need goes
in `display`, written once; input comes in through `display::send`; a software path always works, and the GPU is
only a faster one.

**For the programs' sake (lambda):**
- The accessibility bus (`at-spi2-core`, or toolkits warn), a D-Bus session bus without being asked, `XAUTHORITY`.
- Runtime directories for users besides root (only `/run/user/0` today); `wl_shm` pools in `/dev/shm` and
  `$XDG_RUNTIME_DIR` files, not only memfds.
- Code generated at run time (llvmpipe and lavapipe write theirs with LLVM) in the x86-64 translator: it works
  natively on ARM64.

**Wayland (`os/wayland`):**
- Protocols turned into Rust from their XML by a small generator of our own (no wayland-rs); a connection's messages
  logged, as `WAYLAND_DEBUG` does.
- `wl_surface`'s buffer transform, `offset` and `leave`; subsurfaces composited into their parent, not each a window;
  `wl_touch`'s events.
- `xdg_toplevel.set_app_id`, `xdg_wm_base.ping`, a positioner's `reactive` and parent size, a popup with no xdg
  parent; `xdg_activation_v1`'s token checked.
- `zwp_text_input_v1`, and v3's surrounding text and content type; `zwp_locked_pointer_v1`'s region and hint.
- Fractional scale, viewporter, presentation, idle inhibit, single-pixel buffers and toplevel icons; drag and drop
  and the primary selection; `wlr_layer_shell`.
- A buffer released before the next replaces it, so clients needn't double-buffer; a key and a text commit sent
  together arriving in order; moving a window by its own title bar, tried once `carbon display` can hold a button.

**X11 (`os/x11`):**
- The core protocol: `MIT-MAGIC-COOKIE-1` and big-endian clients; `_NET_WM_PING`, `_NET_WM_SYNC_REQUEST`, struts and
  `_NET_STARTUP_ID`; passive and sync grabs, `WarpPointer`, motion hints; `INCR`, other selection types and XDND;
  dashes, `GraphicsExpose`, and the image's own PCF fonts in place of the one 6×13 font.
- Extensions: SHAPE's `ShapeNotify`, MIT-SHM's pixmaps, RENDER's non-separable blends, alpha maps, filters and indexed
  formats, the rest of XKEYBOARD and RANDR, XFIXES' cursor events and barriers, then DAMAGE, SYNC, XInput 2, the
  Generic Event Extension and COMPOSITE; for the GPU, `Present`, `DRI3` and GLX.
- To run: SDL, a Java Swing program, later Wine's X11 driver.

**Graphics, software first:**
- **In the image:** Mesa's llvmpipe and lavapipe, through `wl_shm` or MIT-SHM: EGL and GLES on Wayland
  (`es2gears_wayland`), GLX (`glxgears`), Vulkan on Wayland and xcb (`vkcube`); then measure whether software is a
  good enough fallback for toolkits and a browser.
- **The host's GPU, shared with atlas:** a neutral, Vulkan-shaped `gpu` module (Metal on the Mac, Vulkan on Linux,
  D3D12 or Vulkan on Windows); a window's buffer as a `gpu` image the desktop composites; a DRM render node,
  `/dev/dri/renderD128`, answering libdrm's and virtio-gpu's ioctls so the image's own Mesa runs unchanged; Mesa's
  Venus stream decoded onto `gpu`, with OpenGL as Zink on top; dmabuf buffers with no copy to the screen. Later,
  VA-API and Vulkan Video.

**Further out:** gamepads, touch and tablets as a neutral part; a desktop session from the image (a panel over
`wlr_layer_shell`), `xdg-desktop-portal` and notifications on the host; sound, with the PulseAudio server.

**Testing:** one scenario through win32k, Wayland and X11 (a framed window, a popup menu, a click, a key, a resize, a
copy and paste) giving the same desktop and the same screenshot; the image's real clients run headless in CI; frame
times for llvmpipe and later Venus.

**Order:** the scenario across the three servers first; XInput 2 and SYNC as toolkits ask, and lambda's part above
alongside; software graphics, then `gpu`; the rest as programs ask.

**Open questions:**
1. *How Vulkan-shaped is `gpu`?* Venus and Direct3D both map well onto Vulkan, with Metal close enough below; exposing
   Vulkan's model itself, or a smaller one both kernels lower to, decides how much each side writes.
2. *Lending pixels in place.* Reading a client's memory at composite time saves a copy a frame, but holds a
   reference into it until release; releasing at once needs a copy at commit, or compositing on commit.
3. *Popups in the host's windows mode*, placed relative to their parent the same way for win32k, Wayland and X11.

### Bugs

- **A multi-process browser's pages never appear.** Its own interface works (tabs, menus, popups, pages the parent
  process draws itself), but every page in a child process shows nothing. The children start, connect to the
  compositor, get vsync and tick, then stop answering the parent on their main IPC channel, so no document is ever
  committed or painted; a headless screenshot hangs too, so it isn't the display server. What it might be, most
  likely first, each with its test:
  1. *Unix stream sockets under load.* The channels are non-blocking `socketpair(AF_UNIX, SOCK_STREAM)` with `epoll`,
     big messages written in pieces as the socket drains, and descriptors passed with `SCM_RIGHTS`. A short write
     with no `EPOLLOUT` after, descriptors dropped on a partial write, or a message misframed across the buffer would
     stall one channel while the rest work. Test: 2–4 MB with a descriptor in one `sendmsg` over a non-blocking pair,
     read with `epoll`, both ways, every byte checked (`net.c` never fills a socket's buffer).
  2. *A lost futex wake-up*, perhaps from vCPU sharing: a wake between a thread giving its vCPU up and blocking again,
     or in `FUTEX_WAKE_OP`/`FUTEX_CMP_REQUEUE` (glibc's condition variables). Test: run with more vCPUs than threads
     and compare; a requeue test.
  3. *The seccomp broker.* A sandboxed child's file access goes through SIGSYS to a broker in the parent; a wrong
     answer (not an error) could wedge a script's loading. Test: the child's sandbox turned off.
  4. *Shared memory between unrelated processes:* memfds passed with `SCM_RIGHTS` and mapped `MAP_SHARED` by a
     forkserver's children (`memory.c` only tests across a fork). Test: a memfd sent to an `exec`ed child, written on
     each side and read on the other, then sealed.
- **gdb can't attach to a multi-threaded process**: it hangs in the attach, and `timeout` can't end it.
- **netlink reports no interface up**, so programs that check say the network is offline.
