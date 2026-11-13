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
  lambda/        lambda         Linux: system calls, processes, memory, files, sockets, /dev and /proc (its README)
  atlas/         atlas          Windows: every system DLL as a declaration, made into PEs in memory (its README)
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

atlas has one more line to draw, as part of Windows has to run inside the program:
[its README](atlas/README.md#part-of-windows-runs-inside-the-program) says where.

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

## Running and testing

Each kernel's README says how to run its programs, how to build its freestanding test programs, and what they check:
[lambda's](lambda/README.md#running-and-testing) and [atlas's](atlas/README.md#running-and-testing). `CARBON_TRACE=1`
prints every system call (lambda) or host export (atlas) with its result, and each thread that gives its CPU up,
waits for one, or takes one again.

## kernels-core

| Module | What it does |
| --- | --- |
| `thread` | A guest thread is a host thread that owns its exec `Cpu`, as its seat: on a vm backend, a thread blocked in a `Waiter` gives its CPU up (registers and space saved) when another thread wants one and none is free, the one blocked longest first, and takes one again when it next needs it, so a machine has more threads than the host gives it vCPUs (64 on an M1). `Handle` preempts a thread whichever CPU it's on. `Cx::turn` is the only way to take exec's lock, held only while its closure runs; `Cx::elsewhere` makes each turn in another space (another process's memory, for `ptrace`), going back to the CPU's own before the turn ends; `run` loops a thread's stops through its kernel's `Guest`, after `Guest::start`; `spawn` starts a new one on a host thread of its own. `Times` is a thread's CPU time, user and system, that its own thread adds to and any thread reads. |
| `timers` | Timers that fire with no thread waiting: one host thread per machine runs each callback when it's due, with nothing locked, so a periodic timer arms its next expiry from its own. lambda's `alarm`, interval timers and POSIX timers use it; atlas's waitable timers and `SetTimer` can. |
| `wait` | One way to wait (`Waiter`: woken, interrupted or timed out), and `Futexes`, keyed by address, for Linux's futexes and Windows' `WaitOnAddress`. |
| `mm` | `Regions<R>`: what a space's program was promised, cut and found by address. lambda's `mmap` and atlas's `VirtualAlloc` both use it. |
| `user` | Copying in and out of program memory, with a bad pointer as a `Fault`. `with_faults` fills a page the program may use (copy-on-write, or not yet touched) the way the program's own touch would, and copies again. |
| `call` | Where a call's arguments are for ARM64, Win64, System V and 32-bit `stdcall`/`cdecl`, and how to set up a call into the program. |
| `frame` | `StackWriter`: building a stack from the top down. |
| `cache` | The page cache: one frame per page of each file, mapped copy-on-write, given back least recently used first. |

To come: `clock`, monotonic and wall time and the page programs read time from (lambda's `vvar`), kept current by a
host thread. atlas's shared data page is kept current on `timers` for now (`system.rs`).

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
| i686 (32-bit x86) | translated by exec: atlas's WoW64 runs on it | native (the shims have a 32-bit code segment; not run yet) |
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

## To do

The order across all of Carbon is in the [top-level README](../README.md#the-order-of-work); this is the kernels'
part of it, in more detail. When an item is done it comes off, and this README says what works.

### kernels-core

- [ ] `clock`: the page programs read time from (lambda's `vvar`, atlas's shared user data), kept current by a host
      thread.
- [ ] Trim the page cache under memory pressure.

### lambda

- [ ] In [its README](lambda/README.md#to-do): system calls, the kernel's gaps, the desktop and graphics, and its
      bugs.

### atlas

- [ ] In [its README](atlas/README.md#to-do), layer by layer: the NT kernel, the window kernel, the services,
      ntdll, the Win32 core, the runtimes and the programs Windows ships.

### obsidian

- [ ] See [its README](obsidian/README.md): settle whether it's its own kernel or lambda with Android's devices.

### Around them

- [ ] A working directory for the first process, from the image's config (lambda starts it in `/`), and the user it
      names.
- [ ] The desktop on Linux and Windows hosts' screens, as on the Mac's (`display/linux`, `display/windows`).
- [ ] Both kernels on the sandboxes in full: a second CPU in each (lambda's fork, atlas's threads), lambda on the
      Windows sandbox (exec's x86 → x86 rewriting), and the test programs on the macOS sandbox.
- [ ] The rest of x86-64 in exec's translator (its README's to-do): x87's transcendentals for libm's `long double`,
      `pushf`/`popf`, signed 128-bit divides, the remaining flags and string instructions, then SSE3 and AVX.
