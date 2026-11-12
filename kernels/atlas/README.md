# atlas

atlas runs Windows programs. It is all of Windows that programs reach, as one Rust module: every system DLL is a
declaration made into a PE in memory, so a machine has no Windows files to install, and the one part of Windows that
has to run inside the program, ntdll, is a crate (`guest/`) that cargo builds for each ISA and embeds.

**Where it stands.** Programs run on ARM64 natively and x86-64 translated, with the loader and SEH, threads and
waits, processes, anonymous and named pipes, sections, asynchronous I/O and completion ports, files, the registry,
security (tokens, descriptors, access checks, impersonation), resources and version information, the system's
version and information, and windows on the desktop with input. Six DLLs declare 501 exports between them, and ntdll
has 75.

## Layout

```
kernels/atlas/
  build.rs        builds guest/ for each ISA and embeds it
  abi/            atlas-abi: what guest/ and atlas share (call numbers, TEB/PEB/CONTEXT layouts, status codes)
  guest/          atlas-guest: ntdll, the in-process part (loader, heap, sync, exceptions and unwinding, environment)
  src/
    lib.rs        boot, the dispatcher
    process.rs threads.rs     processes, threads, the PEB and TEB
    mapping.rs    VirtualAlloc and friends, at 64 KiB granularity
    sections.rs   sections and file mappings
    objects.rs handles.rs     the object manager: handles, names, events, mutants, semaphores, waits, APCs
    files.rs      files and devices over layers, the console
    pipes.rs named_pipes.rs ports.rs    anonymous and named pipes; completion ports and asynchronous I/O
    registry.rs environment.rs
    security.rs   tokens, SIDs, ACLs, security descriptors, SDDL and access checks
    system.rs system_info.rs  the version, KUSER_SHARED_DATA, and NtQuerySystemInformation
    resources.rs  resource directories, string tables and version information
    imports.rs    what a PE imports, against what atlas declares (`carbon atlas imports`)
    exceptions.rs upcall.rs   delivering exceptions; running the program's callbacks from host code
    win32k/       windows and messages over display's desktop, input, GDI
    images/       PE parsing, mapping images, and making the system DLLs from os/'s declarations
    os/           each system DLL as a submodule: kernel32, user32, gdi32, advapi32, …
```

To come beside them: `net/` (Winsock over `net`, DNS, TLS for WinHTTP), bcrypt and ncrypt, `msi/` (Windows Installer
in host Rust, so `msiexec` is a process atlas runs with no files in the machine), sound in `os/winmm`, `os/mmdevapi`
and `os/dsound`, and the system layer's fonts.

## Running and testing

```sh
cargo build --example run-windows
codesign --force --sign - --entitlements carbon.entitlements target/debug/examples/run-windows   # macOS
target/debug/examples/run-windows path/to/program.exe args...
kernels/atlas/tests/programs/build.sh out/      # the test programs, for both ISAs, with LLVM's clang and lld-link
target/debug/examples/run-windows out/loader-aarch64/loader.exe   # DLLs beside it go on the disk too
carbon atlas imports setup.exe                  # what a program imports that atlas lacks (a folder: ranked)
```

The runner uses the host's `vm` backend; `CARBON_ISOLATION=sandbox` (or `auto`) picks the sandbox. ntdll is built
from source for both Windows targets: with rustup, that needs `rustup component add rust-src` and LLVM's `lld-link`
(Debian's `lld` package has it); on Windows, MSVC (Visual Studio Build Tools) and `lld-link.exe`. `CARBON_TRACE=1`
prints every host export with its result, and each thread that gives its CPU up, waits for one, or takes one again.
`cargo test -p atlas` runs the unit tests.

On this Mac, through hvf, every test program passes. Each one's opening comment says what it checks.

| Program | What it covers |
| --- | --- |
| atlas, ARM64 native and x86-64 translated | The basics: the console, the command line, memory, an upcall, a message box, and ntdll's own exports |
| atlas's `windows.c`, both ISAs | Window classes, creation and its messages, painting and GDI, timers, and messages between threads |
| atlas's `desktop.c`, both ISAs | Windows on the desktop: activation and focus, coordinates, input through `SendInput`, capture, the clipboard, stacking |
| atlas's `registry.c`, both ISAs | A fresh machine's hives, keys and every value type, enumeration, deletion, and saving |
| atlas's `environment.c`, both ISAs | The environment a logon gives, from the registry: read, expanded, set and deleted |
| atlas's `files.c`, both ISAs | Folders and files: made, opened, read, found, copied, moved and deleted, and System32's DLLs |
| atlas's `threads.c`, both ISAs | Threads, critical sections and waits, TLS, events, mutexes, semaphores, SRW locks, condition variables and APCs |
| atlas's `loader.c`, both ISAs | An app's own DLLs, relocated, with `DllMain`, forwarders, `LoadLibrary` and `GetProcAddress`, and SEH |
| atlas's `process.c`, both ISAs | Processes and anonymous pipes: `CreateProcess`, redirected output, inherited handles, exit codes |
| atlas's `sections.c`, both ISAs | Sections and file mappings, `VirtualQuery`, and waitable timers |
| atlas's `io.c`, both ISAs | Completion ports and overlapped I/O: events, packets, `ReadFileEx` routines |
| atlas's `pipes.c`, both ISAs | Named pipes, byte and message, pending I/O, and a pipe between two processes |
| atlas's `sysinfo.c`, both ISAs, with and without a manifest | `KUSER_SHARED_DATA`, the version as a program is told it, the machine, time and time zones, `NtQuerySystemInformation` |
| atlas's `security.c`, both ISAs | Tokens and privileges, SIDs, ACLs, descriptors and SDDL, descriptors kept and checked, `AccessCheck`, impersonation |
| atlas's `resources.c`, both ISAs, with `resources.rc` | Finding, loading and enumerating resources, string tables, and version information |

**On Linux and Windows hosts** (x86-64, GCP VMs with nested virtualization), through kvm and winkvm, the first eight
pass on x86-64 (`hello`, `loader`, `threads`, `files`, `registry`, `environment`, `windows` and `desktop`),
`threads.c` with its full 20,000 rounds; the newer ones have been run on macOS. Through the Linux and Windows
sandboxes they pass too, except what needs a second CPU: `threads.c` and the cross-thread part of `windows.c`.

On Windows, atlas's own x86-64 calls into atlas go through its trampoline (`call::TRAP_AT` in the atlas page), which is
`syscall` everywhere but the Windows sandbox, where it's that sandbox's gate; the stubs it makes for host exports do
the same there.

## Windows, in short

Windows' kernel, `ntoskrnl.exe`, has 489 system calls, and its window kernel, `win32k.sys`, 1,485 more, for windows,
input and drawing. Programs never make them: Windows renumbers them nearly every build, so the stable contract is
DLLs' export names (`ntdll` holds the kernel's stubs, `win32u` win32k's, and kernel32, user32 and the rest are built
on them). Of the thousands of DLLs in System32, about 60 carry nearly everything programs import. (Numbers are for
Windows 11 25H2 x64: system calls from j00ru's tables, exports from Wine's `.spec` files.)

atlas runs what Windows runs in the kernel and its service processes as host Rust, and what Windows runs inside the
program in `guest/`, crossing where Windows crosses:

| Layer | Windows | atlas |
| --- | --- | --- |
| 0. CPU | x86-64, ARM64, x86 under WoW64 | exec: ARM64 native, x86-64 translated; i386 not started |
| 1. The NT kernel | `ntoskrnl`, 489 calls | host Rust: objects, threads, processes, memory and sections, files, pipes, completion ports, the registry, security, system information, exceptions |
| 2. The window kernel | `win32k`, 1,485 calls | host Rust: `win32k/`, windows, messages, input and basic GDI, on `display` |
| 3. The services | csrss, lsass, services, rpcss, winlogon, svchost's | none, but the registry |
| 4. ntdll | the loader, heap, Rtl, the Nt stubs | `guest/`: loader, heap, sync, exceptions, environment; 75 exports, two of them `Nt*`/`Zw*` |
| 5. The Win32 core | kernelbase, kernel32, user32, gdi32, advapi32, shell32 | `os/`: six DLLs, 501 exports |
| 6. The runtimes | C runtime, COM, shell UI, graphics, networking, crypto, media, devices | none |
| 7. The programs Windows ships | cmd, reg, regsvr32, rundll32, msiexec, explorer, … | none |

The Win32 core, by atlas's `dll!` declarations (a stub is a `missing` export, declared but ending the process
naming itself):

| DLL | Windows' exports | atlas: host | in ntdll | stubs | declared |
| --- | ---: | ---: | ---: | ---: | ---: |
| kernel32 | 1,561 | 193 | 68 | 1 | 17% |
| user32 | 992 | 99 | | 5 | 10% |
| gdi32 | 545 | 18 | | 5 | 4% |
| advapi32 | 589 | 99 | | 4 | 18% |
| shell32 | 475 | 4 | | | 1% |
| version | 16 | 5 | | | 31% |
| ntdll | 1,544 | | 21 by Windows' names, 54 of atlas's own | | 1% |

Between ntdll and the host there are 12 calls of atlas's own (`atlas-abi`'s `call`), and every host export traps
straight to its Rust.

## How it works

atlas is all of Windows as one Rust module. There's no runtime of DLL files to build separately, install on the
machine's disk and read back, and so no install step and no version handshake between two halves that could come from
different builds: every system DLL is a declaration made into a PE in memory, and the one part that runs in the
program, ntdll, is a single crate cargo builds and embeds. An estimate: 10–15k lines run inside the program; the rest
is host Rust, where it's easier to test, debug and share.

There's no Windows image to bring, so atlas makes the system layer: `C:\Windows`, `System32`
  and `SysWOW64` from `os/`'s declarations, the registry's default hives, and a user profile (fonts to come). The
  user's programs and installers go on top, in the machine's own writable layer. A fresh machine is ready with nothing
  installed.

**For OS DLLs, ours always win.** If an app ships its own `kernel32.dll`, atlas still maps its own, as Windows does
with KnownDLLs. **Ours can still look like files**: `System32\kernel32.dll` shows up in a listing and can be opened and
read, because installers and version checks expect it, but nothing stores it.

### Part of Windows runs inside the program

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

**Processes** (`os/procapi.rs`, `pipes.rs`). `CreateProcessW`/`A` start a process on a host thread and CPU of its
own, with a space of its own, while objects and their names stay the machine's, so processes share them as on
Windows. How it starts:
- **Finding the program**: by its application name, or by the command line's first argument with `.exe` added; then
  the parent's folder, the current directory, System32, Windows and `Path`. There's ERROR_FILE_NOT_FOUND where
  there's none, and ERROR_BAD_EXE_FORMAT for a DLL or anything else that isn't a program.
- **What it starts with**: the command line as given; an environment block (UTF-16 or ANSI, else the parent's as it
  is now); a current directory; and `CREATE_SUSPENDED`, which holds its first thread before its loader runs until
  `ResumeThread`.
- **Handles**: with `bInheritHandles`, every inheritable handle at the same value (`SECURITY_ATTRIBUTES`,
  `SetHandleInformation`, `DuplicateHandle`'s flag). Standard handles come from `STARTUPINFO` with
  `STARTF_USESTDHANDLES`, else the parent's.

It gives process and thread handles to wait on and read exit codes from. `OpenProcess` finds a process by ID,
`TerminateProcess` ends another, and `DuplicateHandle` works between processes. A process that's over lets go of its
handles and memory at once.

Anonymous pipes (`CreatePipe`) are open files, a read end and a write end, that `ReadFile`, `WriteFile`,
`PeekNamedPipe` and `GetFileType` take. A read waits for data, and a write waits for room. When every handle to the
write end is closed in every process, a read of the empty pipe is ERROR_BROKEN_PIPE. When every handle to the read
end is closed, a write is ERROR_NO_DATA. A machine runs until its last process is over, and its exit code is its
first process's.

**Sections** (`sections.rs`, `os/memapi.rs`). `CreateFileMapping` makes a section of the page file or of a file.
A section's frames are atlas's: made as they're first touched, zeros or the file's bytes. Every view of it, in any
process, maps those frames themselves, so all its views see each other's writes. A `FILE_MAP_COPY` view maps them
copy-on-write, so its writes are its own.
- Views may be read-only or writable, as the section and the file's access allow. They start at 64 KiB offsets
  (ERROR_MAPPED_ALIGNMENT otherwise), at a base of the program's or wherever there's room.
- A writable mapping bigger than its file makes the file that big. An empty file with no size is
  ERROR_FILE_INVALID.
- Names are shared by every process, and section handles can be inherited.
- A file mapping's pages go back to the file on `FlushViewOfFile`, on `UnmapViewOfFile`, and as the process ends.

`VirtualQuery` says what any address is: its run of alike pages, its allocation's base, committed, reserved or free,
and private, mapped or image.

**Waitable timers** are objects on `kernels-core`'s timer thread:
- notification timers stay set, and synchronization timers let one wait through;
- a due time is relative or an absolute FILETIME, and a timer may repeat every so many milliseconds;
- `CancelWaitableTimer` stops what's to come;
- an APC routine runs, with its argument and the time, in the setting thread's next alertable wait;
- names are shared.

`GetSystemTimeAsFileTime` gives the wall clock in Windows' units.

**Asynchronous I/O** (`ports.rs`, `os/ioapi.rs`). `ReadFile` and `WriteFile` with an OVERLAPPED do the I/O at its
offset and report how it went, as Windows does:
- The OVERLAPPED's `Internal` gets an NTSTATUS and `InternalHigh` the count.
- Its event is unset while the I/O runs and set when it's done.
- A file opened with `FILE_FLAG_OVERLAPPED` and associated with a completion port gets a packet there, with its
  key, unless `SetFileCompletionNotificationModes` skips it on success.
- A read past the end is ERROR_HANDLE_EOF, with `STATUS_END_OF_FILE` in the OVERLAPPED, and no packet.
- `ReadFileEx` and `WriteFileEx` queue their routine for the thread's next alertable wait.
- `GetOverlappedResult(Ex)` gives the outcome.

Completion ports are queues that threads take packets from (`GetQueuedCompletionStatus(Ex)`, alertable too),
waking the thread that waited last first, and that `PostQueuedCompletionStatus` adds to, so they work as a work
queue too. I/O on the disk completes as it's asked for, as Windows' cached I/O usually does. An overlapped read of a
named pipe with nothing in it stays pending (`ERROR_IO_PENDING`), and another thread completes it:
- **By writing:** the writer, in this process or another, copies the data into the reader's buffer
  (`Cx::elsewhere`, faulting its pages in as the reader's own touch would).
- **By closing:** a close from a call completes it then. When a handle drops as its process ends, with no CPU to
  write with, the event, packet and routine happen at once and the OVERLAPPED is written by the next call into atlas.
- **By cancelling:** `CancelIo` and `CancelIoEx` complete it with ERROR_OPERATION_ABORTED.

**Named pipes** (`named_pipes.rs`, `os/pipeapi.rs`). A server makes `\\.\pipe\NAME` with `CreateNamedPipe`, an
instance at a time, up to its limit (`FILE_FLAG_FIRST_PIPE_INSTANCE` refused when there's one already). A client
connects with `CreateFile` to an instance that's listening (ERROR_PIPE_BUSY when none is), and each connection is
a pipe each way, byte or message.
- **Connecting:** `ConnectNamedPipe` waits for a client, or with an overlapped handle stays pending until a client's
  `CreateFile` completes it. A client that came first gives ERROR_PIPE_CONNECTED; one that came and went gives
  ERROR_NO_DATA.
- **Ending:** a client closing leaves the server what it wrote, then ERROR_BROKEN_PIPE. `DisconnectNamedPipe` ends
  the connection (the client then reads ERROR_PIPE_NOT_CONNECTED), and the instance waits for `ConnectNamedPipe`
  again.
- **Messages:** a message pipe in message read mode gives one message a read, or as much as fits with
  ERROR_MORE_DATA and the rest next time. `PeekNamedPipe` says what's left of the message.
- **The rest:** `WaitNamedPipe`, `SetNamedPipeHandleState` (read mode, `PIPE_NOWAIT`), `GetNamedPipeInfo`, each
  end's process ID, `TransactNamedPipe` and `CallNamedPipe`. Names are the machine's, so processes connect to each
  other's pipes.

Overlapped writes go in at once, however full the pipe is.

**System information** (`system.rs`, `os/sysapi.rs`). atlas is one Windows, 10.0.19045 (22H2), and says so the same
way everywhere: the registry, the PEB, `RtlGetVersion` and the shared data page.
- **`KUSER_SHARED_DATA`:** a read-only page at 0x7ffe0000 in every process, one for each ISA (an ARM64 program and an
  x86-64 one see different processors). It has the version, the processor's architecture and features, the
  processor count, the memory's size, and the clocks: interrupt time, system time and the tick count. The kernel's
  timer thread brings the clocks up to date 64 times a second, as Windows' clock interrupt does, writing each
  `KSYSTEM_TIME` in the order its readers expect. `VirtualProtect` and `VirtualFree` refuse it.
- **The version:** `RtlGetVersion` reads the PEB and tells every program the truth. `GetVersionEx`, `GetVersion`
  and `VerifyVersionInfo` say Windows 8 (6.2.9200) to a program whose manifest doesn't list Windows 10 in its
  `supportedOS`, as Windows does. `VerifyVersionInfo` compares the version and service pack as one number, by the
  condition of the highest part asked for.
- **The machine:** `GetSystemInfo`, `GlobalMemoryStatus(Ex)`, `IsProcessorFeaturePresent`, `IsWow64Process(2)` (no
  WOW64 yet), the processor counts (one group), and `GetLogicalProcessorInformation(Ex)` (a core for each
  processor, two L1 caches each, a shared L2, one package, one NUMA node).
- **Its name and the time:** `GetComputerName(Ex)` (CARBON, and no DNS domain); SYSTEMTIME and FILETIME both ways.
  The time zone is UTC, as the host doesn't say which zone it's in. `SystemTimeToTzSpecificLocalTime` and back follow
  the daylight saving rules of a zone a program gives them.
- **`NtQuerySystemInformation`** (`system_info.rs`), the classes programs use: basic and processor information (and
  their emulation twins), performance and each processor's times (with `GetSystemTimes`), the time of day, the
  process list (an idle process, then every process with its parent, start time, memory, handles and threads), both
  handle lists (each handle's type, and an object address the same in every process for the same object), the
  kernel's modules (`ntoskrnl.exe` and `hal.dll`), the system range and the kernel debugger (none). atlas doesn't
  count the CPU time programs use, so the machine looks idle.

**Resources** (`resources.rs`, `os/resapi.rs`). A module's resources are read where it's loaded, as Windows reads
them: `FindResource(Ex)` walks its resource directory (types, names, then languages), and an `HRSRC` is the address
of its data entry, `LoadResource` the address of its bytes in the module.
- **Finding:** by number, by name in any case, or `#number`; in a language asked for, or with none, neutral, then
  the user's, then US English, then any English, then the first. ERROR_RESOURCE_TYPE_NOT_FOUND, `_NAME_` and
  `_LANG_` say what's missing.
- **Enumerating:** `EnumResourceTypes`, `EnumResourceNames` and `EnumResourceLanguages` call back with named entries
  first, then numbered ones in order; a callback that returns FALSE stops them (ERROR_RESOURCE_ENUM_USER_STOP).
- **Strings:** user32's `LoadString(W/A)` reads string tables, with a size of 0 giving the string's own address.
- **Version information:** version.dll's `GetFileVersionInfo(Size)(Ex)W` read `VS_VERSIONINFO` from a file's bytes,
  through its sections, and `VerQueryValueW` finds the fixed information, strings and translations in it. Every
  DLL atlas makes has a `.rsrc` of its own with version information (10.0.19041.1), so `System32`'s files and the
  loaded modules both have it.

**Security** (`security.rs`, `os/secapi.rs`). There's one user, `carbon` (S-1-5-21-0-0-0-1000, as the registry
has it), an administrator, elevated, at high integrity, as on a machine with UAC turned off.
- **Tokens:** each process has a primary token, a copy of its parent's, so a program's changes to its own (privileges
  enabled, integrity lowered) are its own. `OpenProcessToken`, `GetTokenInformation` (the user, groups, privileges,
  owner, primary group, default DACL, statistics, elevation, integrity, logon SID and the rest),
  `SetTokenInformation`, `AdjustTokenPrivileges`, `PrivilegeCheck`, `DuplicateTokenEx`, `CheckTokenMembership`, and
  the pseudo-handles.
- **SIDs and accounts:** made, compared and copied; well-known SIDs; strings both ways, with SDDL's aliases; and the
  machine's accounts and groups by name both ways (`LookupAccountSid`, `LookupAccountName`).
- **Descriptors and ACLs** are in the program's memory, as on Windows, absolute or self-relative: their parts got and
  set, ACEs added, found and deleted, and SDDL both ways (`ConvertStringSecurityDescriptorToSecurityDescriptor` and
  back), with NULL DACLs and mandatory labels.
- **Kept:** a kernel object or section made with a descriptor in its `SECURITY_ATTRIBUTES` keeps it, as does a new
  file or folder; any of them, and registry keys, take one from `SetSecurityInfo`, `SetNamedSecurityInfo`,
  `SetKernelObjectSecurity`, `SetFileSecurity` and `RegSetKeySecurity`, and give theirs to the `Get` forms. Parts left
  out are filled from the maker's token, and generic rights mapped to the type's own. Files and keys keep theirs in
  memory, for as long as the machine runs. What has none kept has the default: its maker's token's for objects,
  Windows' usual ones for files and keys.
- **Checked:** opening a named object or section, a file, or a key that has a descriptor kept is checked against it,
  as `AccessCheck` is: each right decided by the first ACE that names it, the owner's implicit `READ_CONTROL` and
  `WRITE_DAC`, privileges, and the mandatory label (a token below an object's integrity can't write up to it).
  Changing a descriptor needs `WRITE_DAC` or `WRITE_OWNER` on it, and a SACL SeSecurityPrivilege.
- **Impersonation:** a thread may act as another token (`ImpersonateSelf`, `SetThreadToken`,
  `ImpersonateLoggedOnUser`, `ImpersonateNamedPipeClient`) until `RevertToSelf`. Its effective token is the one
  access is checked with; `OpenThreadToken` and the thread pseudo-handles give it.

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
`SetActiveWindow`, `BringWindowToTop`, `SetWindowPos`'s stacking and activation (all under the foreground rules: a
process whose window isn't active and that didn't have the last input gets a flash, and `SetForegroundWindow` FALSE),
`FlashWindow`, `FlashWindowEx`, `ClipCursor`, `GetClipCursor`, DPI as a program that isn't DPI aware is told it
(96, `GetDpiForWindow`, `GetDpiForSystem`, `GetDeviceCaps`; its windows magnified on a 2× screen), `SetWindowPos`'s `HWND_BOTTOM` and a window to go behind, `WS_EX_NOACTIVATE`, `GetWindow`, `GetTopWindow`,
`EnumWindows` in z-order, files dropped on a window (shell32's `DragAcceptFiles`,
WM_DROPFILES, `DragQueryFileW`, `DragQueryPoint`, `DragFinish`),
`AdjustWindowRectEx`, `GetWindowRect`, `ClientToScreen`, `ScreenToClient` and `WindowFromPoint`.

GDI draws host-side, into each top-level window's pixels on the desktop: DCs from `GetDC` and `BeginPaint`, clipped
to the client area or what needs painting; stock and made brushes and pens; `FillRect`, `FrameRect`, `Rectangle`,
`MoveToEx`/`LineTo`, `SetPixel`/`GetPixel`, `TextOutW` and `DrawTextW` in the one font (5 by 7 in a 6 by 13
cell). The desktop is a screen in memory (1280 by 800, or what `atlas::Options` gives), shown on a Mac with
`carbon run --display host`; with `CARBON_SCREENSHOTS=<folder>`, `run-windows` saves it as `screen.png` as the machine
stops, with each top-level window's last picture beside it.

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

By layer, bottom up. Items are host Rust unless marked **[guest]** (ntdll), or another module's: **[exec]**,
**[display]**, **[gpu]**, **[net]**, **[audio]**.

### 1. The NT kernel

- **Objects:** `SignalObjectAndWait`; keyed events; the object namespace as a tree (directories, symbolic links,
  `Global\` and `Local\`, private namespaces); `NtQueryObject`, with the handle lists' type numbers.
- **Processes and threads:** `Read`/`WriteProcessMemory` and `VirtualAllocEx` across processes; `Get`/
  `SetThreadContext`; `DuplicateHandle` of another process's pseudo-handles; `NtQueryInformationProcess` and
  `NtQueryInformationThread`; job objects (kill-on-close, limits, completion messages); `GetStartupInfo` with what
  the parent passed, and `CREATE_NEW_CONSOLE` and the other console flags; `SuspendThread` on a thread blocked in a
  wait; `DLL_THREAD_DETACH`, `DLL_PROCESS_DETACH` and FLS callbacks as threads and processes end.
- **Memory and sections:** a `WriteFile` to a mapped file seen by its views (one cache for both); `SEC_RESERVE`;
  image sections; guard pages and stack growth, with `STATUS_STACK_OVERFLOW`; `VirtualAlloc2`'s placeholders; write
  watch; large pages, refused politely; one namespace for sections' and other objects' names.
- **Files and I/O:** sharing modes; file times that can be set; hard and symbolic links, junctions and reparse
  points; `ReplaceFile`, and `MoveFileEx` until reboot; change notification (`ReadDirectoryChangesW`); other drive
  letters, `GetVolumeInformation` and free space; overlapped pipe writes that wait for room,
  `GetNamedPipeHandleState`, `CancelSynchronousIo`; mailslots; `DeviceIoControl` for volumes, the console, and AFD
  for sockets **[net]**.
- **The registry:** `RegNotifyChangeKeyValue`; the 32-bit view (`WOW6432Node`); `RegLoadKey`, `RegSaveKey` and
  `RegRestoreKey`; saving more often than at a flush and the machine's end; transactions, refused cleanly.
- **Security:** `SetEntriesInAcl` and the trustee helpers; inheritance from a folder or parent key; descriptors on
  processes, threads, pipes and keys made with one; descriptors kept on disk, not only while the machine runs;
  restricted tokens, as a browser engine's sandbox needs; `MakeAbsoluteSD` and the `*A` functions.
- **IPC:** decide whether RPC between processes is ALPC or a host channel of atlas's own (programs never call ALPC;
  `rpcrt4` and `combase` do).
- **Time and the rest:** the host's own time zone (atlas is UTC); CPU time counted per thread and process, so
  `GetProcessTimes`, the process list and processor times aren't all idle; `QueryInterruptTime` and
  `QueryUnbiasedInterruptTime`; global atoms; NLS's code page and locale tables and sort keys; ETW and debug calls
  that succeed and do nothing, and `OutputDebugString`; power and shutdown, refused or ignored.

### 2. The window kernel

- **NtUser:** the z-order of child windows; `WM_NCHITTEST` and frames a program draws itself; keyboard layouts
  besides US; menus (`CreateMenu`, `SetMenu`, `TrackPopupMenu`, menu bars, accelerators); dialogs (`DialogBox`,
  `CreateDialog`, `IsDialogMessage`); the standard controls (BUTTON, EDIT, STATIC, LISTBOX, COMBOBOX, SCROLLBAR);
  scroll bars and carets; icons and cursors as objects (`LoadImage`, `DrawIconEx`, `GetIconInfo`); hooks
  (`SetWindowsHookEx`); window stations and desktops as objects; `MsgWaitForMultipleObjects`, one wait for messages
  and objects, so no GUI program polls; windows across processes (`SendMessage`, `EnumWindows`, `WindowFromPoint`);
  per-monitor DPI awareness; IME, with imm32.
- **NtGdi:** bitmaps and memory DCs (`CreateCompatibleDC`, `BitBlt`, `StretchBlt`, `CreateDIBSection` on shared
  pages, `AlphaBlend`); regions; ellipses, polygons and paths; mapping modes; real fonts with metrics and the
  registry's substitutes; palettes; printing, refused cleanly.
- **The graphics driver and composition** **[gpu]** **[display]**: behind Direct3D, DXGI and flip-model swap chains;
  `dwmapi`'s queries answered from the desktop.
- **The rest** (input injection, pointer and touch, power), as programs reach it.

### 3. The services

Each becomes host Rust, not a process: **csrss** (process bookkeeping and the console, with conhost's windows);
**lsass** (logons, SSPI with NTLM and schannel, LSA policy); the **service control manager** (`OpenSCManager`,
`CreateService`, start, stop and query, services as atlas processes); **rpcss** (COM activation across processes,
the endpoint mapper, the running object table); **winlogon** (the session and profile, for `userenv`); the
**svchost** services programs reach (the Task Scheduler, the Windows Installer service, CryptSvc, the DNS cache,
BITS, audio **[audio]**, WMI's common classes); and **the event log**.

### 4. ntdll **[guest]**

- **The `Nt*`/`Zw*` exports**, each a stub that traps into atlas: C runtimes, .NET, Go and browser engines call ntdll
  directly. `NtQuerySystemInformation` is the first; next are the ones the kernel already implements behind
  kernel32.
- **`Rtl*`, by family:** strings (`UNICODE_STRING`, conversions, comparisons, case); integers and GUIDs; bitmaps; AVL,
  generic and hash tables; time conversions; security descriptor and SID helpers; compression; the heap's other
  entry points; paths (`RtlDosPathNameToNtPathName_U`); activation contexts.
- **The loader:** `PEB_LDR_DATA`'s three lists, which programs walk themselves; `LdrGetProcedureAddress` and
  `LdrLoadDll`; DLL load notifications; `DllMain` on detach and thread attach; `FreeLibrary` that unloads; delay
  loads (`ResolveDelayLoadedAPI`); API sets from a schema in the PEB (`imports::api_set` knows the map); side-by-side
  manifests.
- The thread pool (`Tp*`) on completion ports; FLS; vectored exception handlers; C++ exceptions' support
  (`__CxxFrameHandler3`/`4`, `_CxxThrowException`) where no vcruntime is brought; the C runtime's slice (`mem*`,
  `str*`, `wcs*`, `sprintf`, `qsort`, `bsearch`); `Etw*` that succeed and do nothing.

### 5. The Win32 core

- **kernel32:** process lists (toolhelp) and `psapi`; the console in full (input records, modes, screen buffers,
  cursor and colours, code pages, `AllocConsole`, virtual terminal sequences); strings and NLS
  (`MultiByteToWideChar` for every code page programs use, `CompareString`, `LCMapString`, `GetLocaleInfoEx`, number
  and date formats, `GetACP`); every `*A` function; resources' rest (`UpdateResource`, `LOAD_LIBRARY_AS_DATAFILE`,
  message tables for `FormatMessage`); fibers; the thread pool's Win32 face; job objects; `SetThreadExecutionState`;
  the long tail, as the import scanner and the `missing` stubs say.
- **advapi32:** services; the event log; CryptoAPI's legacy face over bcrypt; ETW registration that succeeds.
- **user32 and gdi32:** every `*A` function; dialog, menu, icon, cursor and accelerator resources; `wsprintf`;
  `SystemParametersInfo` and the system colours.
- **The shell's core** (shell32, shlwapi, shcore): known folders; `ShellExecute(Ex)` by association and `runas`;
  `SHFileOperation` and `IFileOperation`; shell links; `SHBrowseForFolder`; icons and the system image list;
  `CommandLineToArgvW`; shlwapi's path, string and registry helpers; shcore's DPI and stream helpers.

### 6. The runtimes

OS DLLs are atlas's; redistributables a program ships (vcruntime140, msvcp140, the .NET runtime) are the program's,
running on atlas. Where a group's real surface is COM (Direct3D, DirectWrite, WASAPI, WIC, the shell's objects), each
interface is the to-do, not each export.

| Group | DLLs | To do |
| --- | --- | --- |
| C runtime | ucrtbase, msvcrt | **[guest]** mostly in the program, as library code over kernel32 |
| COM and RPC | combase, ole32, oleaut32, rpcrt4, propsys, urlmon | apartments, the class registry, in-process servers, marshalling, `IStream` and compound files (MSI needs them), OLE automation; WinRT refused cleanly |
| Shell and common UI | comctl32, comdlg32, uxtheme, dwmapi, riched20, imm32, usp10, windowscodecs, gdiplus | comctl32 6's controls and `TaskDialog`, the common dialogs, flat themes, RichEdit, Uniscribe, WIC's decoders, GDI+ |
| Graphics | dxgi, d3d9/11/12, d2d1, dwrite, opengl32, vulkan-1 | **[gpu]** DXGI and Direct3D 11 first with a software path, then 9 and 12, Direct2D, DirectWrite, WGL, Vulkan |
| Networking | ws2_32, mswsock, iphlpapi, dnsapi, winhttp, wininet, netapi32 | **[net]** Winsock with overlapped I/O, `AcceptEx` and `ConnectEx`; addresses; DNS; WinHTTP and WinINet with TLS checked against `host`'s certificates; `URLDownloadToFile` |
| Security and crypto | bcrypt, ncrypt, crypt32, wintrust, secur32 | bcrypt's algorithms first, certificate stores from the host's roots, `WinVerifyTrust`, schannel, DPAPI |
| Media | winmm, dsound, mmdevapi, xaudio2, mf | **[audio]** `waveOut`, `PlaySound` and winmm's timers, WASAPI, DirectSound and XAudio2 over it, later Media Foundation |
| System and devices | setupapi, cfgmgr32, hid, psapi, dbghelp, userenv, winspool, msi, rstrtmgr | psapi, userenv, devices enumerated, hid, dbghelp for crash reporters, printing refused, Restart Manager, msi as a library and `msiexec` |

### 7. The programs Windows ships

First `cmd.exe` (variables, `if`, `for`, `goto`, `call`, pipes, redirection, `.bat` files), `conhost`, `reg.exe`,
`regsvr32.exe`, `rundll32.exe` and `msiexec.exe`; then `sc.exe`, `taskkill`, `tasklist`, `schtasks`, `icacls`,
`where`, `timeout`, `start`, `explorer.exe` and `notepad.exe`; later, enough PowerShell for installers' scripts.

### Across every layer

- **32-bit x86 (WoW64)** **[exec]** **[guest]**: most installers are 32-bit. It needs exec's i386 translation, the
  i686 ntdll, 32-bit TEBs and PEBs, `SysWOW64`, the registry's and file system's redirection, and every layer built
  for i686.
- **ANSI and code pages:** every `*A` function through the same conversions.
- **Manifests:** side-by-side assemblies and activation contexts decide which comctl32 a program gets.
- **API sets:** the loader following `api-ms-win-*` and `ext-ms-*` to their DLL, as the scanner already does.

### What real programs will ask for next

Installers, winget and large desktop apps go past the layers' first items, in roughly this order:

1. **What stops them starting:** `__unDName` (checked against `llvm-undname`) and the C++ runtime's internals;
   `reg.exe`, `regsvr32.exe`, `rundll32.exe` and `cmd.exe`; then `conhost`, `taskkill`, `icacls`, `schtasks`,
   PowerShell for installer scripts, `explorer.exe`.
2. **What ordinary programs use:** menus; real fonts and the rest of GDI, with `DrawTextExW`, `ScrollWindowEx` and
   Uniscribe; real icons and cursors; services; restricted tokens, job objects with limits, and desktops and window
   stations (a browser engine's sandbox); `msi.dll` with compound files written.
3. **What's faked underneath:** `MsgWaitForMultipleObjects`; change notification; sharing modes; links and
   junctions; `MoveFileEx` until reboot; transactions; windows across processes and `SetWindowsHookExW`; registry
   notification, and the environment from the registry so what an installer sets reaches new processes; process
   times, `GetProcessMemoryInfo`, and local time zones.
4. **Packaging:** MSIX, app containers and package identity; WinRT activation.
5. **Graphics, sound, video and input:** Direct3D (dxgi and d3d11, then d3d12 with DXBC and DXIL, d3d9, dcomp),
   Vulkan and OpenGL; Media Foundation's codecs; XInput, `dinput8`, `hid`; `dwrite`'s CFF outlines.
6. **When something asks:** dbghelp and minidumps; printing; serial ports, Bluetooth, DDE; `esent`, `pdh`, `credui`.

Gaps to avoid as these are written: a C runtime with only the C locale, `longjmp` that skips `__finally` blocks and
destructors, COM with one apartment per process, crypto keys only in the process, TLS without ALPN, HTTP/2, proxies or
NTLM, and the x87 stack missing from a 32-bit `CONTEXT`.

### How it's measured, and the order

- **Per DLL:** declared, host, in ntdll and stubs, against Windows' exports, counted from the `dll!` declarations.
- **Per area:** a freestanding test program in `tests/programs` on both ISAs (and i686 once it runs). Still to write:
  `gdi.c`, `controls.c`, `dialog.c`, `com.c`, `winsock.c`, `crypto.c`.
- **The long tail:** `carbon atlas imports` ranks what folders of real programs lack; next is counting the `missing`
  stubs and `GetProcAddress` names programs reach as they run.
- **Checkpoints**, each run headless and screenshotted: a console tool, a plain Win32 program, a dialog program with
  the common controls, an NSIS installer built from source, a networked program, a Direct3D program, a browser
  engine's sandboxed processes.

The kernel's areas come first and are done. Then, each step ending with its tests passing on both ISAs: ntdll's
foundation (the `Nt*` exports, `Rtl*`, the loader's lists, unloading and delay loads, API sets, the thread pool,
FLS); 32-bit, while the layers are thin; the Win32 core's breadth (NLS, the console, every `*A`, advapi32's
services, the shell's core); the window kernel's breadth (bitmaps, fonts, regions, menus, dialogs, controls, icons,
`MsgWaitForMultipleObjects`); the services; the runtimes, group by group; and the programs Windows ships.

### Open questions

- **Win32 over Nt, or Win32 in host Rust?** On Windows, kernelbase's `CreateFileW` runs in the program and calls
  `NtCreateFile`; atlas implements `CreateFileW` itself. Either keep each Win32 function host-side, with one Rust
  implementation behind it and its `Nt*` export, or move the thin wrappers into `guest/` over the `Nt*` stubs, as
  Windows does, so each kernel object has one entry point. The `Nt*` exports are needed either way.
- **RPC between processes:** real ALPC under `rpcrt4`, or a host channel of atlas's own with `rpcrt4` and `combase`
  written to it.
- **ucrtbase:** atlas's own (an OS DLL since Windows 10), or Microsoft's where a program brings one.
- **Which Windows atlas is:** it reports Windows 10 22H2 (10.0.19045); Windows 11 would change manifests, API sets and
  what programs check for.
- **Graphics APIs.** Direct3D, DXGI and OpenGL make thousands of calls a frame, and as `host` exports each would be
  an exit. Real drivers batch calls in the program and submit once a frame. atlas probably needs the same: record
  commands into a buffer in the program, and translate them to Metal or Vulkan host-side at `Present`. That's a
  second, small piece of code in the program. (Host exports, guest code, or a GPU protocol: not decided.)
- **How much of the C runtime runs in the program.** `printf` and `strtod` are big and called often, while `fopen`
  should trap. Measuring real programs should draw this line, not a guess made up front.

Settled while building:

- **Where the registry is saved**: one file in the machine's own layer, `C:\Windows\System32\config\REGISTRY`, in
  atlas's own format. It goes with the machine and freezes into an image with it.
- **The toolchain for `atlas/guest`**: rustup's prebuilt `core` for the Windows targets links but leaves `memcpy`,
  `memset`, `memcmp` and `strlen` undefined, and ntdll has no C runtime beneath it to supply them, so it's built with
  `build-std` and `compiler-builtins-mem`, always (`RUSTC_BOOTSTRAP`; rustup needs `rust-src`, Homebrew's Rust
  has the source). The same build runs on macOS, Linux and Windows.
