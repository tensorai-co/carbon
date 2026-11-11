# kernels

Carbon's kernels, on top of [`exec`](../exec/README.md): **lambda** runs Linux programs and **atlas** runs Windows
programs. A third, **[obsidian](obsidian/README.md)** for Android, is a placeholder.

A kernel is a Rust module, like `exec`. So is everything an operating system brings with it: lambda's `/proc`, vDSO
and display servers, and atlas's `kernel32`, `user32`, registry and window manager. A program sees
`C:\Windows\System32\kernel32.dll` because atlas makes it up in memory, the same way lambda makes up
`/proc/self/maps`.

```
kernels/
  core/          kernels-core   what both share, with nothing of Linux or Windows in it
  lambda/        lambda         Linux: system calls, processes, memory, files, sockets, /dev and /proc
  atlas/         atlas          Windows: every system DLL as a declaration, made into PEs in memory
    abi/         atlas-abi      what atlas and the code inside programs share
    guest/       atlas-guest    ntdll: the part of Windows that runs inside programs, built per ISA and embedded
  obsidian/                     Android: not started
```

## The rule: the kernel brings the OS, the image brings the user's software

Nothing that belongs to an OS is a file Carbon builds, installs or reads back. Debian brings glibc, bash and apt; an
installer brings an app.

| Ours: never a file of its own | Theirs: real files, used as they are |
| --- | --- |
| Both kernels | Everything an image or an app brings |
| atlas's `ntdll` (`atlas/guest`, embedded in Carbon's binary) and every OS DLL, made from its `dll!` declaration | An app's own DLLs, and the redistributables it ships (`vcruntime140.dll`, `msvcp140.dll`): Microsoft's real code, running on our `ntdll` |
| The registry's defaults, `msiexec`, the system layer's directories | Installers, and whatever they install |
| lambda's `/proc`, `/sys`, `/dev`, vDSO and display servers | glibc or musl, `ld-linux`, every `.so`, bash, apt |

**For OS DLLs, ours always win.** If an app ships its own `kernel32.dll`, atlas still maps its own, as Windows does
with KnownDLLs. **Ours can still look like files**: `System32\kernel32.dll` shows up in a listing and can be opened and
read, because installers and version checks expect it, but nothing stores it.

### The catch: part of Windows has to run inside the program

On Windows, `kernel32.dll` isn't a service the program calls; it's code inside the program. `HeapAlloc`,
`EnterCriticalSection`, `TlsGetValue` and `memcpy` are called millions of times a second, and a trip to the kernel costs
microseconds on `vm` and tens of microseconds on `sandbox`, a thousand times more. The OS calls back (`WndProc`,
`EnumWindows`, `qsort`), and those calls nest. SEH and C++ exceptions unwind through system DLL frames. Programs read
the TEB, PEB, loader lists and heaps directly. Linux doesn't have this problem: the syscall is the line, and the
image's own libc does everything in-process.

So the rule for atlas is **cross where Windows itself crosses**. Code Windows runs in the program stays in the program
(`atlas/guest`). Code Windows runs in its kernel (`ntoskrnl`, `win32k`) or in a service process (`csrss`, `lsass`,
`services`, the registry, the spooler) becomes host Rust in atlas. Microsoft drew that line for performance and
safety, so programs already expect a call across it to cost something.

### Around the kernels

Kernels depend on the modules beside them, and nothing outside `machine/` depends on a kernel:

| Module | What a kernel uses it for |
| --- | --- |
| [`exec`](../exec/README.md) | Running the program's code, its memory and CPUs |
| [`layers`](../layers/README.md) | The file system programs see: an image's index plus the machine's own log. Each kernel's metadata (`M`) is kept there but never read, and saved as four words (`layers::Persist`) |
| [`display`](../display/README.md) | The desktop: the window manager both kernels share. win32k, Wayland and X11 put top-level windows on it, and it frames, stacks and focuses them and routes input back |
| [`audio`](../audio/README.md) | The mixer every program's sound goes into, and the microphone: lambda's PulseAudio server and atlas's winmm, WASAPI and DirectSound play into it (to do) |
| [`net`](../net/README.md) | Every socket: the machine's addresses, loopback, Unix sockets and DNS |
| [`host`](../host/README.md) | Standard streams, clocks, randomness, CPUs, memory, the terminal, certificate trust |

Each kernel's display server stays in the kernel (lambda's `os/wayland` and `os/x11`, atlas's `win32k`); the window
manager under them is one, `display::Desktop`. [`remote`](../remote/README.md) reads the desktop and the mixer and
serves them to another computer; no kernel knows it's there. [`images`](../images/README.md) is `machine`'s; no
kernel touches it.

`machine` boots a kernel with `lambda::boot` or `atlas::boot`, and stops it from another thread through the `Stop` in
its `Options`: lambda's signals process 1 or ends everything, atlas's ends its program.

- **lambda's machine**: the root is the image's layers (Debian's, say) plus `os/`'s file systems. lambda loads the
  command, its ELF interpreter from the image, and its signal trampoline (later, the vDSO) from memory. A static
  program needs no image: `run ./static-binary` runs on an empty root plus `os/`.
- **atlas's machine**: there's no Windows image to bring, so atlas makes the system layer: `C:\Windows`, `System32`
  and `SysWOW64` from `os/`'s declarations, the registry's default hives, and a user profile (fonts to come). The
  user's programs and installers go on top, in the machine's own writable layer. A fresh machine is ready with nothing
  installed.

## Running something

```sh
# A static Linux program (ARM64 runs natively on Apple silicon; x86-64 is translated):
cargo build --example run
codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run
target/debug/examples/run path/to/static-binary args...

# A Windows program:
cargo build --example run-windows
codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run-windows
target/debug/examples/run-windows path/to/program.exe args...
```

Both runners use the host's `vm` backend; `CARBON_ISOLATION=sandbox` (or `auto`) picks the sandbox. On Linux, kvm
needs the user in the `kvm` group. atlas's ntdll is built from source for both Windows targets: with rustup, that
needs `rustup component add rust-src` and LLVM's `lld-link` (Debian's `lld` package has it).

**On Linux and Windows** (x86-64, GCP VMs with nested virtualization), through kvm and winkvm, every test program
below passes on x86-64: lambda's hello (12), signals (11), `sse`, `x87`, `threads` (9, its 300 threads and 300 processes
included), `code`, `events` (36) and `net` (36), and all eight of
atlas's, `threads.c` with its full 20,000 rounds. There `sse` and `x87` run on the real CPU, which confirms their answers. Through the
Linux and Windows sandboxes they pass too, except what needs a second CPU, which the sandboxes don't have yet:
lambda's fork and wait checks, and atlas's `threads.c` and the cross-thread part of `windows.c`. lambda doesn't run in the Windows sandbox
yet, since its programs' `syscall` needs rewriting into calls to that sandbox's gate.

On Windows, atlas's own x86-64 calls into atlas go through its trampoline (`call::TRAP_AT` in the atlas page), which is
`syscall` everywhere but the Windows sandbox, where it's that sandbox's gate; the stubs it makes for host exports do
the same there. Building on Windows needs MSVC (Visual Studio Build Tools) and LLVM's `lld-link.exe`.

### Debian

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

`CARBON_TRACE=1` prints every system call (lambda) or host export (atlas) with its result.

Each kernel has freestanding C test programs for both ISAs, built with Homebrew's LLVM (lambda's all build like `hello.c`):

```sh
# lambda: no libc, just system calls
clang --target=aarch64-linux-gnu -nostdlib -static -ffreestanding -fno-stack-protector -fno-builtin \
    -fuse-ld=lld -O2 kernels/lambda/tests/programs/hello.c -o hello-aarch64
clang --target=x86_64-linux-gnu -nostdlib -static -ffreestanding -fno-stack-protector -fno-builtin \
    -fuse-ld=lld -O2 kernels/lambda/tests/programs/hello.c -o hello-x86_64

# atlas: no C runtime; hello-*.exe, and loader-*/ with loader.exe and its DLLs
kernels/atlas/tests/programs/build.sh out/
target/debug/examples/run-windows out/loader-aarch64/loader.exe   # DLLs beside it go on the disk too
```

On this Mac, through hvf, all of them pass:

| Program | Checks |
| --- | --- |
| lambda, ARM64 native and x86-64 translated | argv, getpid, uname, brk and mmap with pages faulted in on touch, files read from and written to the disk, getdents64, clocks, pipes, fork with copy-on-write, a child writing to a pipe, wait4 |
| lambda's `signals.c`, both ISAs | a handler with siginfo and `rt_sigreturn`, blocked and pending signals, a SIGSEGV handler that moves the program on, SIGTERM to a child, SIGSTOP/SIGCONT/SIGKILL as `wait4` reports them, `SA_RESTART` and EINTR, SIGPIPE and EPIPE, a pty's line editing and echo, ^C on a pty |
| lambda's `threads.c`, both ISAs | four threads by `clone`, each with its own tid and thread pointer (`CLONE_SETTLS`), joined by `CLONE_CHILD_CLEARTID`; a futex mutex contended by four threads for 20,000 rounds each, and an atomic add; futex wait and wake, 2,000 turns between two threads; 300 threads and then 300 processes, each ended before the next, more than a machine has CPUs at once |
| lambda's `code.c`, both ISAs | code written at run time, as JITs do: a function written to a writable, runnable page and rewritten 1,000 times; a direct call into code that's then rewritten; W^X (written, `mprotect`, run, and again); `munmap`, then new code at the same address; another thread rewriting a function this one keeps calling; 4,096 functions rewritten 16 times while another thread runs its own. On Apple silicon the x86-64 build checks that the translator forgets what it translated; `CARBON_EXEC_CODE_CACHE=65536` makes its code cache move between halves too |
| lambda's `events.c`, both ISAs | eventfd counts, `EFD_SEMAPHORE`, EAGAIN, and a blocking read woken by another process; timerfd one-shot and interval expiries, `timerfd_gettime`, disarming; epoll level-triggered, edge-triggered and one-shot, `EPOLLOUT`, `EPOLLHUP`, EEXIST/ENOENT/EPERM, a timeout, a wait woken by another process, an edge-triggered timerfd, a closed file leaving, an epoll inside another and polled, `epoll_pwait2`; a signalfd woken in `poll` and `epoll` by another process's `kill`; the signal masks of `ppoll` and `epoll_pwait`; x86-64's `epoll_create`, `epoll_wait` and `eventfd` |
| lambda's `net.c`, both ISAs | `socketpair` both ways, `FIONREAD` and `MSG_PEEK`, `shutdown` and `POLLRDHUP`, EPIPE and `POLLHUP`; a pipe passed with `SCM_RIGHTS` and read from the other side; seqpacket and `MSG_TRUNC`; a Unix server at a path (a socket file: EADDRINUSE to bind again, ENXIO to open) accepting from another process, with `SO_PEERCRED` and `getsockname`; ECONNREFUSED and ENOENT; Unix datagrams to a path from an abstract name; TCP on 127.0.0.1 listed in `/proc/net/tcp`, an echo from another process after a half-close, `getsockname`/`getpeername`, `TCP_NODELAY`, ECONNREFUSED, a non-blocking accept and connect to the machine's own address with epoll and `SO_ERROR`, a reset; UDP with its sender, `sendmmsg`/`recvmmsg`, `SO_RCVTIMEO`; raw sockets refused; netlink's links and `SIOCGIFINDEX`; `/etc/resolv.conf` and `/etc/hosts`; the machine's DNS server answering its own name |
| lambda's `files.c`, both ISAs | a hard link and `st_nlink`, the file read by its second name once the first is gone; `chmod` (still a regular file), `chown` with and without -1, `utimensat`'s modification time, all as `stat` sees them; a file removed while open, written and read; `flock`; `F_SETLK` and `F_GETLK`'s `F_UNLCK`; `close_range`, with `CLOSE_RANGE_CLOEXEC` and without |
| atlas, ARM64 native and x86-64 translated | console handles, `WriteConsoleW`, the command line, `VirtualAlloc` committed on touch, an upcall into the program (`EnumSystemLocalesW`, twice), `MessageBoxW`, and the guest exports: the heap, the last error and critical sections, running inside the program in the embedded ntdll |
| atlas's `windows.c`, both ISAs | `RegisterClassExW`; `CreateWindowExW` with WM_NCCREATE and WM_CREATE (and its parameter) and WM_SIZE (its client area, inside the frame `AdjustWindowRectEx` says); window text through DefWindowProc's WM_GETTEXT and WM_SETTEXT; `FindWindowW`; `SendMessageW` on the window's thread; window and class longs; subclassing with `CallWindowProcW`; a child window, `EnumChildWindows`, `EnumWindows`; `ShowWindow`, `UpdateWindow`, WM_ERASEBKGND and WM_PAINT; `FillRect`, the class background, a pen's `Rectangle` and `LineTo` read back with `GetPixel`; `MoveWindow` and WM_SIZE; `SendMessageW` and `PostMessageW` from another thread into a `GetMessageW` loop; timers; `PeekMessageW`; WM_CLOSE destroying a window and its child; `PostQuitMessage` |
| atlas's `desktop.c`, both ISAs | windows made hidden, then shown and activated; WM_ACTIVATE, WM_SETFOCUS and WM_KILLFOCUS; `GetWindowRect` with the frame and `GetClientRect` inside it; `ClientToScreen`, `ScreenToClient`, `WindowFromPoint`, `GetSystemMetrics`; through `SendInput`: a click (WM_LBUTTONDOWN in client coordinates, activating and raising), Shift+A, b and a Unicode `!` typed as WM_CHAR "Ab!" with `GetKeyState` as of each message, the wheel, dragging a title bar (WM_MOVE) and a corner (WM_SIZE), another window's close button; a popup with no frame, closed by Alt+F4; `SetForegroundWindow` |
| atlas's `registry.c`, both ISAs | a fresh machine's HKLM (`ProductName`, a DWORD by `RegGetValueW`, a type not asked for, `REG_EXPAND_SZ` expanded); keys made and opened in another case; every value type and the default value; sizes and `ERROR_MORE_DATA`; `RegSetValueExA` storing UTF-16; `RegQueryInfoKeyW`; enumerating values and keys; names keeping their case; deleting values, keys (not with subkeys) and trees; a deleted key's handle; `HKEY_CLASSES_ROOT` as `HKLM\SOFTWARE\Classes`; `RegFlushKey` saving it |
| atlas's `environment.c`, both ISAs | `SystemRoot`, `PROCESSOR_ARCHITECTURE`, `Path` from the registry expanded, `USERNAME` through the A version, `NUMBER_OF_PROCESSORS`, `TEMP`; ERROR_ENVVAR_NOT_FOUND; set and get in another case; sizes when the buffer's short, missing or just enough; replacing; `SetEnvironmentVariableA`; an empty value; `=` in a name refused; `ExpandEnvironmentStrings` (W and A) with unknown names and a short buffer; `GetEnvironmentStringsW` in order with the new variables; the ANSI block; deleting with NULL |
| atlas's `files.c`, both ISAs | the current directory; folders made, refused when there, and removed; `CREATE_NEW`, `OPEN_EXISTING`, `OPEN_ALWAYS` and `CREATE_ALWAYS` with their last errors; reads, seeks, a read at the end, sizes, `SetEndOfFile`; names in any case; `GetFinalPathNameByHandleW` in the disk's own case; attributes, and `ERROR_FILE_NOT_FOUND` against `ERROR_PATH_NOT_FOUND`; relative paths and `GetFullPathNameW`; `FindFirstFileW`/`FindNextFileW` with wildcards and `.`; copy, move, replace, delete; `ERROR_DIR_NOT_EMPTY`; delete-on-close; the temp folder; System32's `kernel32.dll` read as a PE from memory, not writable, and listed with ntdll; `NUL` and `CONOUT$` |
| atlas's `threads.c`, both ISAs | four threads and a contended critical section with wait-all; `GetExitCodeThread`; `TlsAlloc` slots and `__declspec(thread)`, each thread's own; `DLL_THREAD_ATTACH`; auto- and manual-reset events; a wait that times out; semaphores and their maximum; a mutex owned, waited for, released, not owned, and abandoned; an SRW lock and a condition variable; `WaitOnAddress`; `InitOnceExecuteOnce` from four threads; `CREATE_SUSPENDED` and `ResumeThread`; `QueueUserAPC` and an alertable `SleepEx`; named events; `TerminateThread`; `ExitThread`; `DuplicateHandle` of a pseudo-handle |
| atlas's `loader.c`, both ISAs | an app's own DLL, imported and relocated off a base the program has; its `DllMain`; a forwarder chain (`mylib!heap_alloc` → `kernel32!HeapAlloc` → `ntdll!RtlAllocateHeap`); `LoadLibraryA` and `GetProcAddress` by name and by ordinal; `ERROR_MOD_NOT_FOUND` and `ERROR_PROC_NOT_FOUND`; `GetModuleFileNameW`; static TLS and a TLS callback; `__try`/`__except` catching an access violation, from three calls down too; `RaiseException` with a filter; `__finally` during an unwind and on a normal exit; `EXCEPTION_CONTINUE_EXECUTION`; integer divide by zero (x86-64) |

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

## kernels-core

| Module | What it does |
| --- | --- |
| `thread` | A guest thread is a host thread that owns its exec `Cpu`. `Cx::turn` is the only way to take exec's lock, held only while its closure runs; `run` loops a thread's stops through its kernel's `Guest`; `spawn` starts a new one on a host thread of its own. |
| `wait` | One way to wait (`Waiter`: woken, interrupted or timed out), and `Futexes`, keyed by address, for Linux's futexes and Windows' `WaitOnAddress`. |
| `mm` | `Regions<R>`: what a space's program was promised, cut and found by address. lambda's `mmap` and atlas's `VirtualAlloc` both use it. |
| `user` | Copying in and out of program memory, with a bad pointer as a `Fault`. `with_faults` fills a page the program may use (copy-on-write, or not yet touched) the way the program's own touch would, and copies again. |
| `call` | Where a call's arguments are for ARM64, Win64, System V and 32-bit `stdcall`/`cdecl`, and how to set up a call into the program. |
| `frame` | `StackWriter`: building a stack from the top down. |
| `cache` | The page cache: one frame per page of each file, mapped copy-on-write, given back least recently used first. |

To come: `clock`, monotonic and wall time and the page programs read time from (lambda's `vvar`, atlas's shared user
data), kept current by a host thread.

### The run loop

Every guest thread, in lambda and atlas alike, runs the same loop. exec has two rules that shape it: only a CPU's own
thread can take its turn, and `Exec::enter` takes `&mut Exec`, so exec sits behind one lock.

```rust
loop {
    let exit = cpu.run();            // no lock held: other CPUs run at the same time
    // 1. A short turn: decode the stop, and answer it there if that's quick (brk, mprotect, a fault filled in),
    //    or copy the arguments in.
    // 2. No locks: the slow part. Files, sockets, waits, the window manager, upcalls.
    // 3. A short turn again: copy results out and set the return value.
}
```

**A turn is short, and nothing slow happens in one.** Reading a file, waiting on a futex or running an upcall never
holds exec, so a thread that blocks never holds up another thread's page fault. One kernel lock around everything
would let a blocked syscall hold up everyone's page faults.

**Locks**, in the order they're taken: a kernel's process table (read-mostly), one process (its handles or file
table, its address-space map, its pending signals or APCs), a shared object (a pipe, the futex table, the disk, the
page cache, a window), and exec's last, only for a turn. Nothing is taken after exec's, and no lock is held across a
wait or an upcall.

## CPU architecture

| A program in | On ARM64 hosts | On x86-64 hosts |
| --- | --- | --- |
| ARM64 | native | needs ARM64 → x86-64 translation (not started) |
| x86-64 | translated by exec | native |
| i686 (32-bit x86) | translated by exec (32-bit decoding not started) | native (the shims need a 32-bit code segment) |
| ARM32 | not supported: Apple silicon can't run it, and few programs need it | not supported |

exec handles all translation, so **a kernel only ever sees the program's own ISA**, and `kernels-core::call` is the one
place that knows what each register means. Beyond that, a kernel's ISA-specific code is limited to layouts (CONTEXT
for x86, x64 and arm64; the TEB and PEB at 32 and 64 bits; signal frames; `ucontext_t`), what the CPU says it has
(CPUID's answers and `AT_HWCAP`, claiming only what exec can run: SSE2 and no AVX for translated x86 today), and the
builds of `atlas/guest`.

**Pages.** Programs get the page size their OS promises: 4 KiB pages for x86 Linux and Windows, plus Windows' 64 KiB
allocation granularity. On `vm` backends this costs nothing. The macOS sandbox protects memory in 16 KiB pages, so
`kernels-core::mm` has to accept a coarser protection than a program asked for, and say when that breaks something,
such as a guard page inside a 16 KiB page.

## lambda

For lambda, the syscall is the line between program and kernel, and the image brings glibc or musl, the
dynamic loader and everything above them. What lambda brings is what a Linux kernel provides beyond syscalls, all of it
host Rust.

```
kernels/lambda/src/
  lib.rs          boot, the dispatcher
  abi.rs          Linux's structs and constants (private: nothing in the program needs them)
  sys/            each ISA's numbers mapped to one enum at the edge: arm64.rs, x86_64.rs (i386.rs to come)
  process/        clone, execve, loading ELF, credentials
  signal.rs       delivery, sigreturn, each ISA's signal frame
  mm.rs           mmap, mprotect, brk, mremap, madvise
  fs/             the file table over layers: pipes, poll, epoll, eventfd, timerfd, metadata
  net/            the socket syscalls over net, netlink, /proc/net
  time.rs tty.rs  clocks and timers; the line discipline, the console and ptys
  os/             what Linux itself provides, made in memory: /dev, /proc
                  to come: sysfs, the vDSO, wayland/ and x11/ pulse/ (a PulseAudio server)
```

Each ISA's system call numbers become one `Sys` at the edge (`sys/`), with x86-64's old calls (`open`, `stat`, `fork`)
turned into their modern forms and `clone`'s arguments put in one order, so nothing past it knows the ISA. i386 will
fix its own quirks (`mmap2`) there the same way.

- **Processes** (`process/`): `execve` with `#!` scripts, ELF loading with an interpreter and position-independent
  programs, the first stack with its auxiliary vector; `clone` for threads and processes (fork is exec's
  `fork_space`; a new thread joins its process before it can run, so one that ends at once is never left listed);
  `wait4` with `WUNTRACED` and `WCONTINUED`; exit and orphans; futexes; sessions and process groups; credentials
  (`creds.rs`: real, effective, saved and filesystem IDs and supplementary groups, with Linux's rules, so a program
  that drops root, as `apt` does to `_apt`, can't get it back; files' permissions aren't checked against them yet).
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
  `rt_sigreturn` through a trampoline page (`[vdso]`) when there's no `SA_RESTORER`, `SA_RESTART`, `SA_ONSTACK` with
  `sigaltstack`, `SA_NODEFER`, `SA_RESETHAND`, `rt_sigsuspend`, `rt_sigpending`, `signalfd` (woken when a signal it
  reads becomes pending), the signal mask of `ppoll`, `pselect6` and `epoll_pwait`, SIGCHLD with its code and
  status, SIGPIPE, and faults as SIGSEGV, SIGILL, SIGBUS, SIGFPE and SIGTRAP.
- **Terminals** (`tty.rs`): one line discipline (canonical and raw, `VMIN`/`VTIME`, erase, word-erase, kill, EOF,
  echo with `ECHOCTL`, `ONLCR`, signal keys) for the console and for pty pairs; `TCGETS`/`TCSETS*`, window sizes,
  foreground groups, controlling terminals, `TIOCGPTN`, `TIOCSPTLCK`, `TIOCGPTPEER`; SIGTTIN for background reads;
  SIGHUP when a session leader leaves.
- **Memory** (`mm.rs`): `mmap` (anonymous and file-backed through the page cache), `munmap`, `mprotect`, `mremap`,
  `madvise`, `brk`. Mapping only makes a region; pages are filled when the program faults on them.
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
- **The OS** (`os/`): `/dev` put on the disk at boot, and `/proc/self/{exe,cwd,fd,maps,status}`, `cpuinfo`,
  `meminfo`, `uptime` made as they're read.

## atlas

atlas is all of Windows as one Rust module. There's no runtime of DLL files to build separately, install on the
machine's disk and read back, and so no install step and no version handshake between two halves that could come from
different builds: every system DLL is a declaration made into a PE in memory, and the one part that runs in the
program, ntdll, is a single crate cargo builds and embeds. An estimate: 10–15k lines run inside the program; the rest
is host Rust, where it's easier to test, debug and share.

```
kernels/atlas/
  build.rs        builds guest/ for each ISA and embeds it
  abi/            atlas-abi: what guest/ and atlas share (call numbers, TEB/PEB/CONTEXT layouts, status codes)
  guest/          atlas-guest: ntdll, the in-process part (loader, heap, sync, exceptions and unwinding, environment)
  src/
    lib.rs        boot, the dispatcher
    process.rs threads.rs     processes, threads, the PEB and TEB
    mapping.rs    VirtualAlloc and friends, at 64 KiB granularity
    objects.rs handles.rs     the object manager: handles, names, events, mutants, semaphores, waits, APCs
    files.rs      files and devices over layers, the console
    registry.rs environment.rs
    exceptions.rs upcall.rs   delivering exceptions; running the program's callbacks from host code
    win32k/       windows and messages over display's desktop, input, GDI
    images/       PE parsing, mapping images, and making the system DLLs from os/'s declarations
    os/           each system DLL as a submodule: kernel32, user32, gdi32, advapi32, …
```

To come beside them: `net/` (Winsock over `net`, DNS, TLS for WinHTTP), `security/` (tokens, SIDs, access checks only
as deep as programs need; bcrypt and ncrypt), `msi/` (Windows Installer in host Rust, so `msiexec` is a process atlas
runs with no files in the machine), sound in `os/winmm`, `os/mmdevapi` and `os/dsound`, and the system layer's fonts.

Every system DLL is a `dll!` declaration in `os/`, and each export is one of three kinds:

```rust
dll! {
    "KERNEL32.dll";
    host WriteConsoleW(5) => write_console_w;   // host Rust behind a trap stub
    guest HeapAlloc => RtlAllocateHeap;         // forwarded to atlas-guest's ntdll
    missing CreateFiber;                         // a stub that says it isn't written yet
}
```

From that, `images/` makes the DLL's PE for each ISA in memory: headers, a `.text` of trap stubs and an export table
with forwarders. A host export's stub puts its call number in `x8` (ARM64) or `eax` (x86-64, with the first argument
moved to `r10` as Windows does) and traps; atlas reads the arguments by the ISA's convention, runs the Rust, and sets
the return value. A guest export forwards into ntdll, as Windows' own `kernel32!HeapAlloc` forwards to
`ntdll!RtlAllocateHeap`. A missing one ends the process naming itself (`kernel32!CreateFiber`), so the next thing to
write is obvious. An image is made the first time a process needs it for an ISA, then shared read-only by every
process, the way the page cache shares a file. COM objects atlas implements will be host-side too: a vtable of trap
stubs per interface, so `IStream::Read` traps into Rust like any `host` export.

**Upcalls.** A host export runs on the thread that owns the program's CPU, so it can run the program again, nested,
from inside the same Rust function, as Windows' `win32k` calls a `WndProc` (`KeUserModeCallback`): `upcall` pushes a
frame whose return address is a trap stub in ntdll, runs the CPU as usual (the callback can make calls, fault, or
upcall again), and returns the callback's value when it comes back to the stub. Three rules:

- **An exception never unwinds through host frames.** The upcall frame has a handler that stops the unwind and
  reports it, as Windows does at its own kernel-callback boundary; `longjmp` across it is treated the same.
- **Upcalls nest only so deep**: a runaway `SendMessage` loop ends the process instead of overflowing Carbon's stack.
- **No lock is held across an upcall.**

lambda doesn't need upcalls: a Linux signal handler is a frame on the program's stack, and leaves through
`rt_sigreturn`, just another syscall.

**What runs in the program** (`atlas/guest`) is what has to: the loader (it walks the program's memory and calls its
code), heaps, the uncontended path of every lock, SEH and unwinding (they walk the program's stack), the TEB, PEB, TLS
and `GetLastError` (programs read them directly), the trampolines where atlas enters the program, and, to come, the C
runtime's hot half (`mem*`, `str*`, `printf`'s formatting). It's a real PE per ISA, so unwinding through it works.

`atlas-guest` is `no_std` Rust built by atlas's `build.rs` for `aarch64-pc-windows-msvc` and `x86_64-pc-windows-msvc`,
linked with LLVM's `lld-link`, and embedded with `include_bytes!`. `core` is always built from source (see the open
questions, below). Where it can't be built, atlas says so and won't start Windows programs.

**Starting a process.** atlas maps the program and ntdll (relocating either if its base is taken), lays out the PEB,
process parameters, TEB and stack, and starts the first thread in ntdll's `AtlasProcessStart`. From there, as on
Windows, it's the loader in the program (`guest/src/loader.rs`):

- It resolves the program's imports, asking atlas to map each DLL by name (`MAP_IMAGE`). atlas's own come from their
  declarations, always; anything else is found as Windows looks, in the program's folder and then System32, and
  relocated as it's mapped, as NT's kernel does.
- Exports by name (binary search) and by ordinal; forwarders (`dll.name`, `dll.#ordinal`) followed to the end; the
  import table opened with `PROTECT` while it's filled in.
- Static TLS for each module with a TLS directory (its index, and this thread's block from its template), TLS
  callbacks, then each DLL's `DllMain`, dependencies first, then the program's entry point.
- `LoadLibraryA/W`, `LoadLibraryExA/W`, `GetProcAddress`, `GetModuleHandleA/W`, `GetModuleFileNameW` and
  `FreeLibrary` are kernel32 guest exports that forward to it.

**Threads.** Each thread is a guest thread with a CPU, a TEB and a stack of its own (`threads.rs`). It starts in
ntdll's `AtlasThreadStart`, which waits there if it was made suspended, gives the thread its own static TLS blocks,
calls each DLL's `DllMain` with `DLL_THREAD_ATTACH`, then runs the thread's routine. `SuspendThread` preempts a
thread, which parks on its way back to its program; `TerminateThread` and `ExitProcess` end threads at their next
chance, waking them from any wait. A process ends with its last thread.

**Objects and waits** (`objects.rs`, `os/synch.rs`): events, mutexes (with ownership, recursion and abandonment),
semaphores, and thread and process objects, signalled when they end; names, which `Create*` and `Open*` share;
handles, `CloseHandle`, `DuplicateHandle`, and the `-1` and `-2` pseudo-handles. One dispatcher lock covers waits and
signals, as NT's does, so `WaitForMultipleObjects` with `bWaitAll` takes every object at once. Waits time out, and
alertable ones run queued APCs (`QueueUserAPC`) as upcalls, returning `WAIT_IO_COMPLETION`.

**In the program** (`guest/src/sync.rs`): critical sections, SRW locks, condition variables, `InitOnceExecuteOnce`
and `WaitOnAddress` never leave the program uncontended; contended, they wait in atlas by address (`WAIT_ADDRESS`,
`WAKE_ADDRESS`, on `kernels-core`'s futexes). `TlsAlloc` slots are in the TEB, FLS slots in a block it points at,
and `GetCurrentThreadId` and friends read the TEB. The exception dispatch in progress is per thread, in the TEB.

**Files** (`files.rs`, `os/fileapi.rs`): Windows paths (`C:\...`, relative to the current directory, root-relative,
`\\?\`, either slash, `.` and `..`, trailing dots and spaces dropped) onto the machine's disk, which folds case.
`CreateFileW`/`A` with every disposition, access checks, folders by backup semantics, delete-on-close, and the `NUL`,
`CON`, `CONIN$` and `CONOUT$` devices; `ReadFile` and `WriteFile` at the file's position or an `OVERLAPPED` offset;
`SetFilePointer(Ex)`, `GetFileSize(Ex)`, `SetEndOfFile`, `GetFileType`, attributes and `GetFileInformationByHandle`;
folders, delete, move, copy; `FindFirstFile`/`FindNextFile` with `*` and `?`; the current directory, full and temp
paths. Errors are Windows' own: ERROR_FILE_NOT_FOUND where the folder's there, ERROR_PATH_NOT_FOUND where it isn't.

The **system layer** puts `C:\Windows`, `System32`, `SysWOW64` and `Temp` on every disk. `System32` also has atlas's
DLLs and ntdll, which nothing stores: listing it shows them, and opening one reads the image atlas made in memory for
the program's ISA. They can't be written or deleted. The loader's `MAP_IMAGE` finds them the same way.

**The registry** (`registry.rs`, `os/advapi32.rs`): a tree of keys and values in atlas, whose names fold case and
keep it. A fresh machine has Windows' usual keys: the version under `HKLM\SOFTWARE\Microsoft\Windows NT`, the
folders, the environment, the processor, the user's shell folders and locale. `HKEY_CURRENT_USER` is the user's key
under `HKEY_USERS`, and `HKEY_CLASSES_ROOT` is `HKLM\SOFTWARE\Classes`. It's saved, but for volatile keys, in the
machine's own layer at `C:\Windows\System32\config\REGISTRY` (atlas's own format, where Windows keeps its hives),
when `RegFlushKey` asks and when the machine stops, and loaded from there when it starts. advapi32 has `RegOpenKeyEx`,
`RegCreateKeyEx`, `RegQueryValueEx`, `RegSetValueEx` (W and A), `RegGetValueW` with its type filters and expansion,
deleting values, keys and trees, enumerating keys and values, `RegQueryInfoKeyW`, `RegFlushKey` and `GetUserNameW`.

**The environment** (`environment.rs`, ntdll's `env.rs`): atlas makes a process's first environment as Windows does
at logon: the system `Environment` key under `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager`, then the
user's (whose `Path` goes after the system's), expanded; the session's variables (the user's folders and name, the
computer's name, `SystemRoot`), `PROCESSOR_ARCHITECTURE` for the program's ISA and `NUMBER_OF_PROCESSORS`; then any
the machine was started with (`Options::env`). It goes in the process parameters as Windows' block of UTF-16
`NAME=VALUE` strings, sorted by name, and from there the program works on it in ntdll, with no call into atlas:
`GetEnvironmentVariable`, `SetEnvironmentVariable` (a NULL value deletes; each change makes a new block on the
process heap, kept in order), `ExpandEnvironmentStrings` (names it doesn't know stay as they are),
`GetEnvironmentStrings` and `FreeEnvironmentStrings`, each in W and A. Names fold case.

**Windows** (`win32k/`, `os/user32.rs`, `os/gdi32.rs`): window classes and windows, with parents and children,
text, longs and extra bytes, and places and sizes; each thread's message queue. A window belongs to the thread that
made it, and its `WndProc` runs on that thread by upcall: `SendMessageW` from that thread calls it at once, and from
another queues it and waits, taking messages sent to its own windows meanwhile, as Windows does. `GetMessageW` and
`PeekMessageW` take sent messages first, then posted ones, WM_QUIT, WM_PAINT for windows that need painting, and
timers. `DefWindowProcW` creates, closes, keeps text, erases backgrounds and validates. Creating and destroying send
WM_NCCREATE, WM_CREATE, WM_SIZE, WM_MOVE, WM_DESTROY and WM_NCDESTROY in Windows' order.

**The desktop.** Top-level windows go on the machine's desktop (`display::Desktop`), the window manager lambda will
share. It has their pixels, draws their frames (a title bar with the window's title and a close button, and edges to
resize by), stacks them (always-on-top above the rest, owned windows above their owners), decides which is active,
and routes the pointer and keys. win32k keeps what Windows says (a window's rectangle is frame and all, its client
area inside; child windows; the focus) and turns the desktop's events into messages as each thread looks at its
queue: WM_ACTIVATE, WM_SETFOCUS and WM_KILLFOCUS; WM_MOVE and WM_SIZE when the user drags a frame; WM_SYSCOMMAND's
SC_CLOSE from the close button; WM_MOUSEMOVE (merged while unread), the buttons and WM_MOUSEWHEEL, to the child window
under the pointer; WM_KEYDOWN and WM_KEYUP, or WM_SYSKEY* with Alt, to the focus. Overlapped windows get a caption, as
on Windows; popups have no frame. `ShowWindow` activates as its command says, `DefWindowProcW` focuses on WM_ACTIVATE,
closes on SC_CLOSE and Alt+F4, and `TranslateMessage` types WM_CHAR on a US layout. `GetKeyState` is the thread's key
state as of the messages it has taken, `GetAsyncKeyState` the keys as they are. `SendInput` (mouse, keys, scan codes
and Unicode characters), `SetCursorPos` and `GetCursorPos` work on the desktop as the user would; so do
`SetForegroundWindow`, `SetActiveWindow`, `BringWindowToTop`, `SetWindowPos`'s stacking and activation,
`AdjustWindowRectEx`, `GetWindowRect`, `ClientToScreen`, `ScreenToClient` and `WindowFromPoint`.

GDI draws host-side, into each top-level window's pixels on the desktop: DCs from `GetDC` and `BeginPaint`, clipped
to the client area or what needs painting; stock and made brushes and pens; `FillRect`, `FrameRect`, `Rectangle`,
`MoveToEx`/`LineTo`, `SetPixel`/`GetPixel`, `TextOutW` and `DrawTextW` in the one font (5 by 7 in a 6 by 13
cell). Nothing is shown on the host yet: the desktop is a 1280 by 800 screen in memory (or one `atlas::Options` gives),
and with `CARBON_SCREENSHOTS=<folder>`, `run-windows` saves it as `screen.png` as the machine stops, with each
top-level window's last picture beside it.

**Exceptions.** A fault, a bad instruction, a breakpoint or a divide by zero is delivered by atlas to ntdll's
`AtlasDispatchException`, with an EXCEPTION_RECORD and a CONTEXT (Windows' layouts, in `atlas-abi`) on the program's
stack. ntdll does the rest as Windows does (`guest/src/exception.rs`, `unwind.rs`):

- It walks up the stack by each function's `.pdata` and unwind data: x64's `UNWIND_INFO` (with chained entries, and
  epilogs recognised by their code), and ARM64's `.xdata` codes (with `save_next`, epilog scopes and packed epilogs)
  and packed entries. The establisher frame is the frame pointer where a prolog sets one, as clang's and MSVC's
  funclets expect.
- Each language handler is asked in turn. `__C_specific_handler` runs `__try` scopes: filters, `__except` targets
  through `RtlUnwindEx` (which calls `__finally` handlers on the way, and continues at the target with the code in
  the result register), and `EXCEPTION_CONTINUE_EXECUTION`.
- `NtContinue` is atlas's `CONTINUE` call. `RtlRaiseException` and kernel32's `RaiseException` raise from the
  program itself, with `RtlCaptureContext` in assembly. `RtlLookupFunctionEntry`, `RtlVirtualUnwind`, `RtlUnwind`,
  `RtlUnwindEx` and `SetUnhandledExceptionFilter` are there for runtimes that use them.
- Unhandled, it says which exception and where, and the process ends with its code.

---

## To do

The order across all of Carbon is in the [top-level README](../README.md#the-order-of-work); this is the kernels'
part of it, in more detail. When an item is done it comes off, and this README says what works.

### kernels-core

- [ ] `clock`: the page programs read time from (lambda's `vvar`, atlas's shared user data), kept current by a host
      thread.
- [ ] Trim the page cache under memory pressure.

### lambda

- [ ] Signals: queued real-time signals (today each is pending once), `rt_sigtimedwait`, `setitimer` and `alarm`,
      SIGTTOU, the FP state in the signal frame itself (it's kept beside it today), and core dumps.
- [ ] `execve` with other threads running (end them first); `vfork` that really shares memory.
- [ ] Shared file mappings written back; `msync`.
- [ ] `EPOLLEXCLUSIVE` and `EPOLLWAKEUP` are accepted and ignored.
- [ ] A console on Windows hosts (raw mode is Unix-only for now), and a full-screen editor tried on it.
- [ ] A vDSO made in memory for each ISA, reading kernels-core's clock page.
- [ ] `clone3`, `rseq`, process groups and sessions as job control needs them; files' permissions checked against
      credentials (the credentials themselves are done).
- [ ] sysfs, and more of `/proc`.
- [ ] Wayland and X11 servers, as `os/wayland` and `os/x11` answering sockets in the machine.
- [ ] A PulseAudio server, as `os/pulse` (its native protocol), on `audio`.
- [ ] Locks that hold between processes (`flock`, `fcntl`'s record and OFD locks).
- [ ] `apt install` on the Linux and Windows hosts, natively on x86-64 (it works on the Mac, in both images).
- [ ] i386 programs: `sys/i386.rs`, with exec's 32-bit translation.
- [ ] x86-64 calls not there yet: the old `getdents`, `utime`, `utimes` and `futimesat`, and
      `MAP_32BIT`; `/proc/cpuinfo` and ptrace's register sets per ISA.

### atlas

- [ ] `CreateProcess`, with handle inheritance and pipes, and objects shared between processes. Installers start
      other programs constantly.
- [ ] The loader's lists in the PEB (`PEB_LDR_DATA`), which some programs walk themselves; `DllMain` on detach and
      thread attach; `FreeLibrary` that unloads; delay-loaded imports; API sets (`api-ms-win-*`); side-by-side
      manifests.
- [ ] C++ exceptions: `__CxxFrameHandler3`/`4` and `_CxxThrowException` (vcruntime's, when a program brings it, should
      work on what's here; atlas's own are still to write). Vectored exception handlers. Stack overflow as
      `STATUS_STACK_OVERFLOW` with a guard page.
- [ ] FLS; the C runtime's hot half (`mem*`, `str*`, `printf`'s formatting) in ntdll.
- [ ] Waitable timers, I/O completion ports (with files), fibers, the thread pool, `SignalObjectAndWait`.
- [ ] `DLL_THREAD_DETACH` and `DLL_PROCESS_DETACH`, and FLS callbacks, as threads and processes end.
- [ ] `SuspendThread` on a thread that's blocked in a wait (it parks when the wait ends, not before).
- [ ] File mappings (`CreateFileMapping`, `MapViewOfFile`) and sections shared between views; asynchronous I/O and
      completion ports; sharing modes; file times that can be set; `ReplaceFile`, hard and symbolic links, reparse
      points; other drive letters; `GetVolumeInformation` and disk space.
- [ ] The registry: `RegNotifyChangeKeyValue`, security (`RegGetKeySecurity`), the 32-bit view (`WOW6432Node`),
      `RegLoadKey` and `RegSaveKey`, and saving more often than at a flush and the machine's end.
- [ ] The desktop, the rest: minimizing and maximizing; the cursor's shape; `SetCapture`; the z-order of child
      windows; WM_NCHITTEST and frames a program draws itself; keyboard layouts besides US; the clipboard.
- [ ] Menus; the standard controls (BUTTON, EDIT, STATIC, LISTBOX); dialogs and resources (`LoadString`,
      `DialogBox`, icons and cursors); ANSI (`*A`) versions; `MsgWaitForMultipleObjects`.
- [ ] GDI: bitmaps and memory DCs (`CreateCompatibleDC`, `BitBlt`, `CreateDIBSection` with shared pages), regions,
      ellipses and polygons, real fonts.
- [ ] 32-bit x86: the i686 build of ntdll, 32-bit TEB and PEB, `SysWOW64`, and exec's 32-bit translation, which
      installers need.
- [ ] Winsock over `net`: `ws2_32` and `mswsock` with overlapped I/O through completion ports, `AcceptEx` and
      `ConnectEx`; `iphlpapi`, `dnsapi`, `IcmpSendEcho`; then WinHTTP and WinINet, with TLS checked against `host`'s
      certificates. A `winsock.c` test program.
- [ ] Sound: winmm (`waveOut`), WASAPI (`mmdevapi`), DirectSound, on `audio`.
- [ ] `msiexec` in host Rust, and the programs installers run: `cmd.exe`, `reg.exe`, `regsvr32.exe`, `rundll32.exe`.
- [ ] The long tail of exports, as programs ask for them: the `missing` stubs say which.
- [ ] A coverage tool: run a folder of real packages and installers, and count which
      `missing` exports they reach, so the long tail is ordered by evidence rather than guesses.

**What real programs will ask for next.** Installers, winget and large desktop apps (a browser engine's, say) go
well past the items above. In rough order:

1. **What stops programs starting**: `__unDName` (MSVC's demangler, plain safe Rust, checked against `llvm-undname`)
   and the C++ runtime's internals (`__BuildCatchObject`, `__TypeMatch`, `__CxxExceptionFilter`, …); the programs
   installers run (`reg.exe` with `add`, `delete`, `query`, `export`, `import`; `regsvr32.exe`; `rundll32.exe`;
   `cmd.exe` with variables, `if`, `for`, `goto`, `call`, pipes, redirection and `.bat` files), then `conhost`,
   `taskkill`, `icacls`, `schtasks`, enough PowerShell for installer scripts, and `explorer.exe`.
2. **What ordinary programs and installers use**: menus (`TrackPopupMenu`, `CreateMenu`, `SetMenu`, …); real fonts
   and the rest of GDI (`GetCharWidthW`, `GetOutlineTextMetricsW`, shapes, mapping modes, palettes, regions), with
   `DrawTextExW`, `ScrollWindowEx` and `SetScrollInfo`, then Uniscribe shaping more than left-to-right text; real icon
   and cursor objects (`DrawIconEx`, `GetIconInfo`, `LoadImage`) and the shell's system image list; Windows services
   (`OpenSCManager`, `CreateServiceW`, `StartServiceCtrlDispatcherW`, a `services.exe` and `sc.exe`); security for
   real: tokens with privileges, groups, restricted SIDs and integrity levels, kept security descriptors and SDDL, job
   objects with kill-on-close and limits, desktops and window stations, which is what a browser engine's sandbox
   needs; `msi.dll` as a library (with `ole32`'s compound files written), `rstrtmgr`, `wldap32`, `sxs`, `ktmw32`.
3. **What's faked underneath**: one wait covering window messages and sockets (`MsgWaitForMultipleObjects`), so no GUI
   program polls; file change notification; share modes enforced; reading symbolic links and junctions;
   `MoveFileEx` until reboot; transactions; cross-process windows (`SendMessage` to another process, `EnumWindows`
   and `WindowFromPoint` across processes, `SetWindowsHookExW`); registry change notification, and the environment
   from the registry so what an installer sets (`PATH`) reaches new processes; thread and process times,
   `GetProcessMemoryInfo`, `GetNativeSystemInfo` following the machine's CPUs, and local time zones.
4. **Packaging and modern programs**: MSIX and the deployment service, app container profiles, package identity; WinRT
   (`combase`, `RoGetActivationFactory`, `WindowsCreateString`, activation from the registry).
5. **Graphics, sound, video, input**: Direct3D (`dxgi` and `d3d11` first, then `d3d12` with DXBC and DXIL, then `d3d9`,
   `dxva2`, `dcomp`), Vulkan and OpenGL with EGL; sound (above); Media Foundation's codecs; XInput, `dinput8`, `hid`,
   `winusb`, `setupapi`'s device calls; `dwrite`'s CFF outlines.
6. **When something asks**: `dbghelp`'s symbols and stack walking, `MiniDumpWriteDump` that writes a dump, event
   tracing; printing (`winspool`, `StartDocW`, enhanced metafiles, the print and font dialogs); serial ports,
   Bluetooth, DDE; `esent`, `pdh`, `credui`, `cryptui`.

Gaps to avoid as these are written: a C runtime with only the C locale, `longjmp` that skips `__finally` blocks and
destructors, COM with one apartment per process, crypto keys only in the process, TLS without ALPN, HTTP/2, proxies or
NTLM, and the x87 stack missing from a 32-bit `CONTEXT`.

### obsidian

- [ ] See [its README](obsidian/README.md): settle whether it's its own kernel or lambda with Android's devices.

### Around them

- [ ] A working directory for the first process, from the image's config (lambda starts it in `/`), and the user it
      names.
- [ ] The desktop on the host (`display/macos`, then Linux and Windows): the whole desktop in one host window, or
      each top-level window in a native one; `carbon display` screenshots, clicks and keys.
- [ ] Both kernels on the sandboxes in full: a second CPU in each (lambda's fork, atlas's threads), lambda on the
      Windows sandbox (exec's x86 → x86 rewriting), and the test programs on the macOS sandbox.
- [ ] The rest of x86-64 in exec's translator (its README's to-do): x87's transcendentals for libm's `long double`,
      `pushf`/`popf`, signed 128-bit divides, the remaining flags and string instructions, then SSE3 and AVX.

## Open questions

1. **Graphics APIs.** Direct3D, DXGI and OpenGL make thousands of calls a frame, and as `host` exports each would be
   an exit. Real drivers batch calls in the program and submit once a frame. atlas probably needs the same: record
   commands into a buffer in the program, and translate them to Metal or Vulkan host-side at `Present`. That's a
   second, small piece of code in the program. (Host exports, guest code, or a GPU protocol: not decided.)
2. **How much of the C runtime runs in the program.** `printf` and `strtod` are big and called often, while `fopen`
   should trap. Measuring real programs should draw this line, not a guess made up front.

Settled while building:

- **Where the registry is saved**: one file in the machine's own layer, `C:\Windows\System32\config\REGISTRY`, in
  atlas's own format. It goes with the machine and freezes into an image with it.
- **The toolchain for `atlas/guest`**: rustup's prebuilt `core` for the Windows targets links but leaves `memcpy`,
  `memset`, `memcmp` and `strlen` undefined, and ntdll has no C runtime beneath it to supply them, so it's built with
  `build-std` and `compiler-builtins-mem`, always (`RUSTC_BOOTSTRAP`; rustup needs `rust-src`, Homebrew's Rust
  has the source). The same build runs on macOS, Linux and Windows.
