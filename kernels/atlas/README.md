# atlas

atlas runs Windows programs. It is all of Windows that programs reach, as one Rust module: every system DLL is a
declaration made into a PE in memory, so a machine has no Windows files to install, and the one part of Windows that
has to run inside the program, ntdll, is a crate (`guest/`) that cargo builds for each ISA and embeds.

**Where it stands.** Programs run on ARM64 natively and x86-64 translated, and 32-bit x86 programs run too (WoW64,
translated on Apple silicon; phases 1 to 5 of its six, below, are done): 30 of the test programs pass every check as
32-bit programs, with x86's own SEH, PE32 images and 32-bit layouts throughout, and a real 32-bit installer starts,
unpacks its data, runs its start-up script, shows its dialogs, installs its files with its progress shown, and finishes. The program it installs then starts, downloads its
240 MB update over HTTPS (Winsock on `net`, its own TLS, crypt32 checking the server's certificates against the
host's trust), verifies, unpacks and installs it, and starts the full program it updated to, whose own imports are
next. They have the loader and SEH, threads and
waits, processes, anonymous and named pipes, sections, asynchronous I/O and completion ports, files, the registry,
security (tokens, descriptors, access checks, impersonation), resources and version information, the system's
version and information, and windows on the desktop with input. The loader is Windows' own in all but manifests: the
PEB's lists, references counted and DLLs unloaded, detach as threads and the process end, API sets and delay loads.
Exceptions go to vectored handlers before frames' handlers, and guard pages and stack overflows raise theirs. Fibers
switch in the program, each with its own stack and FLS, whose callbacks run as fibers, threads and slots end. Files
are shared as their opens say, with hard and symbolic links and times that can be set. The thread pool runs in the
program: work, timers and waits, on pools of their own or the process's. The registry tells watchers of changes, has
WOW64's 32-bit view, and saves, restores, loads and unloads hives. Code pages convert as Windows' do, every `*A`
function speaks Windows-1252, and ANSI windows get their messages' text in it. ucrtbase.dll is atlas's own C runtime, in the
program, so MinGW-built C programs run. Fifteen DLLs declare 1,260 exports between them, 1,241 written and 19 stubs that say so, and ntdll has 332. A DLL's exports can have Windows' own ordinals,
gaps and all, which programs import oleaut32's, ws2_32's and some of shell32's by.

## Layout

```
kernels/atlas/
  build.rs        builds guest/ for each ISA (ARM64, x86-64 and i686) and embeds it
  abi/            atlas-abi: what guest/ and atlas share (call numbers, TEB/PEB/CONTEXT layouts at 64 and 32 bits,
                  status codes)
  guest/          atlas-guest: ntdll, the in-process part (loader, heap, sync, exceptions and unwinding, environment);
                  seh32.rs is x86's SEH, msvc32.rs its 64-bit division helpers
  src/
    lib.rs        boot, the dispatcher
    abi.rs        a process's structures by its ISA: TEB, PEB, parameters, records, OVERLAPPED, pointer width
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
    images/       PE parsing, mapping images, making the system DLLs from os/'s declarations (PE32+, or PE32 with
                  stdcall stubs), and where things go in each ISA's space (`layout`)
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
kernels/atlas/tests/programs/build.sh out/      # the test programs, for each ISA, with LLVM's clang and lld-link
target/debug/examples/run-windows out/loader-aarch64/loader.exe   # DLLs beside it go on the disk too
target/debug/examples/run-windows out/loader-i686/wow64.exe       # a 32-bit program
carbon atlas imports setup.exe                  # what a program imports that atlas lacks (a folder: ranked)
```

The runner uses the host's `vm` backend; `CARBON_ISOLATION=sandbox` (or `auto`) picks the sandbox. 32-bit programs
run on hvf so far (kvm and winkvm haven't run them yet; the sandboxes don't run them). ntdll is built from source
for all three Windows targets: with rustup, that needs `rustup component add rust-src` and LLVM's `lld-link`
(Debian's `lld` package has it); on Windows, MSVC (Visual Studio Build Tools) and `lld-link.exe`. `CARBON_TRACE=1`
prints every host export with its result, and each thread that gives its CPU up, waits for one, or takes one again.
`cargo test -p atlas` runs the unit tests.

On this Mac, through hvf, every test program passes, on ARM64 and x86-64, and the 28 built for i686 too. Each one's
opening comment says what it checks. `crt.exe` (and `crt-mingw.exe`) want the arguments `one "two words"`.

| Program | What it covers |
| --- | --- |
| atlas, ARM64 native and x86-64 translated | The basics: the console, the command line, memory, an upcall, a message box, and ntdll's own exports |
| atlas's `menus.c`, both ISAs and i686, with `menus.rc` | Menus: built with `AppendMenu` and read back by position and by command (into a submenu too); checks, radio checks, enabling, the default item; `InsertMenuItem`, `GetMenuItemInfo`, `SetMenuItemInfo`, `RemoveMenu`; `MENU` and `MENUEX` resources; `SetMenu` and `GetMenu`; `TrackPopupMenu` driven by another thread's `SendInput`: the arrows past a separator and Enter, Escape, an `&` letter, a submenu by its letter, a click, and `WM_COMMAND` without `TPM_RETURNCMD`; `WM_INITMENUPOPUP` |
| atlas's `images.c`, both ISAs and i686, with `images.rc` (`images.bmp`, `images.ico`, `images.cur`) | Bitmaps and icons: `LoadBitmap` and `GetObject`; a memory DC with a bitmap selected, read and drawn into; `BitBlt` and `StretchBlt` between it and a window both ways, and `BLACKNESS`; `CreateCompatibleBitmap`; `LoadImage` choosing an icon's image by size and scaling one, and `DrawIconEx` and `DrawIcon` blending by alpha and by mask; `LoadCursor`; a bitmap from a file, scaled; what isn't there, and a system icon; statics showing an icon their template names and a bitmap `STM_SETIMAGE` gives; an image list cut from a bitmap with a colour key, an icon added, drawn |
| atlas's `shell.c`, both ISAs and i686 | The shell: shortcuts (a shell link set through `IShellLinkW`, saved through `IPersistFile` as a `.lnk` file whose header is read back, loaded into another and read through `IShellLinkW` and `IShellLinkA`, its item ID list, `QueryInterface` for what it isn't, reference counts); special folders' paths (Program Files as the program's ISA sees it), their item ID lists and the paths back, a virtual folder's, `ILCreateFromPath`; `SHFileOperation` copying a tree, a wildcard into a folder it makes, renaming, moving and deleting; `ShellExecuteEx` running a program and waiting for its exit code, `ShellExecute`, and what it can't open; `SHGetFileInfo`'s names, attributes, image list and program type; `SHBrowseForFolder` with its callback, accepted, cancelled, and refusing a path that isn't a folder |
| atlas's `comctl.c`, both ISAs and i686 | comctl32's controls in a dialog: a progress bar's range, position, delta, step, clamping and bar colour, and what it paints; a report list view's columns, items inserted where asked, their text in each column and their `lParam`, `LVM_GETITEM`, selection and the `LVN_ITEMCHANGED` its parent hears, single selection, the down arrow, a column sized to its text, `LVM_ENSUREVISIBLE`, deleting, its image lists and its procedure through `CallWindowProc`; image lists made, added to, counted, sized, removed from and destroyed |
| atlas's `dialog.c`, both ISAs and i686, with `dialog.rc` | Dialogs: the dialog class as `GetClassInfoEx` gives it and a class of the program's own from it; templates in memory and a `DIALOGEX` resource, their controls, IDs and text; `WM_INITDIALOG` and the focus; `DWLP_DLGPROC`; `Get`/`SetDlgItemText` and `Int`; check boxes and an automatic radio group; `WM_COMMAND` and `EN_CHANGE`; `DM_GETDEFID`; Tab, Enter, Escape and typing through `IsDialogMessage`; `EnableWindow`; a control's procedure through `CallWindowProc`; what the controls paint, read with `GetPixel`, in a `WM_CTLCOLORSTATIC` brush; modal dialogs, their owner disabled, ended by Enter and by `WM_CLOSE` |
| atlas's `windows.c`, both ISAs | Window classes, creation and its messages, painting and GDI, timers, and messages between threads |
| atlas's `desktop.c`, both ISAs | Windows on the desktop: activation and focus, coordinates, input through `SendInput`, capture, the clipboard, stacking |
| atlas's `crt.c`, both ISAs, and built by MinGW-w64 (x86-64) | The C runtime: printf's conversions, snprintf, strings, strtol and strtod, qsort, the heap, getenv, the arguments, files in text and binary mode, rand, atexit |
| atlas's `ansiwin.c`, both ISAs | ANSI windows: a class and window made with the `*A` functions, its `WndProc` given names, text and characters in 1252 however they're sent; a Unicode window driven by the `*A` functions; `GetMessageA` and `DispatchMessageA`; `WndProc`s set by `SetWindowLongPtrA` and `W` |
| atlas's `ansi.c`, both ISAs | Code pages: `MultiByteToWideChar` and `WideCharToMultiByte` in 1252, 437 and UTF-8; `*A` functions with é in their names, as the `*W` ones see them: files, paths, the registry, the environment, objects, accounts, locales and version information |
| atlas's `registry.c`, both ISAs | A fresh machine's hives, keys and every value type, enumeration, deletion, and saving; change notification, asynchronous and not; the 32-bit view; hives saved, restored, loaded and unloaded |
| atlas's `sockets.c`, both ISAs and i686 | Winsock over the machine's loopback: TCP's listen, connect, accept and names; send, recv, MSG_PEEK, FIONREAD; `select` and `WSAPoll`; non-blocking sockets and a connect seen through `select`; SO_RCVTIMEO; options; overlapped `WSARecv` through an event and a completion port, done at once and later, with and without FILE_SKIP_COMPLETION_PORT_ON_SUCCESS; `closesocket` aborting what's under way; shutdown; UDP; `ConnectEx`, `AcceptEx` and `GetAcceptExSockaddrs` from `WSAIoctl`; `getaddrinfo` (numeric, passive, localhost, the machine's own name, AI_NUMERICHOST); `gethostname`, `inet_addr`; ws2_32's old functions by ordinal |
| atlas's `startup.c`, both ISAs and i686 | What larger programs call as they start: the heap's sizes, reallocation and queries; encoded pointers; `InitOnceBeginInitialize`/`Complete` and `ExecuteOnce`; interlocked lists; `IsDebuggerPresent`, `DebugBreak` to a vectored handler, `UnhandledExceptionFilter`; `IsBadWritePtr`/`ReadPtr`; `CompareString`, `LCMapString` (case and sort keys), `GetStringType`; `GetDateFormat`/`GetTimeFormat`; disk space, drive types, `FindFirstFileEx`; console modes and `SetStdHandle`; `OpenThread`, priorities, affinity, the session; psapi; the monitor, the window station, `MapWindowPoints`, `MsgWaitForMultipleObjects` (a timeout, a message, an object); memory fonts; the event log and `RtlGenRandom`; `CommandLineToArgvW`, `SHGetKnownFolderPath`, `IsUserAnAdmin`; oleaut32's BSTRs and VARIANTs, bcrypt's random bytes and Winsock's start, imported by ordinal |
| atlas's `installer.c`, both ISAs and i686 | What installers call first: the `lstr*` functions, `CharNext`/`CharPrev`, `MulDiv`, `CompareFileTime`; `wsprintf` with its arguments past the registers, cut at 1024 characters, and `wvsprintf`; temporary and short names, disk space, `SearchPath`; .ini files; the error mode; COM's start, its task allocator and `CoCreateInstance` with no class registered; `SHGetFileInfo` with no flags |
| atlas's `environment.c`, both ISAs | The environment a logon gives, from the registry: read, expanded, set and deleted |
| atlas's `links.c`, both ISAs | Sharing modes, opens refused and allowed; deleting and moving open files; a deleted file still read where it's open; hard links; symbolic links to files and folders, as attributes, listings and `RemoveDirectory` see them; file times set and read back |
| atlas's `files.c`, both ISAs | Folders and files: made, opened, read, found, copied, moved and deleted, and System32's DLLs |
| atlas's `threads.c`, both ISAs | Threads, critical sections and waits, TLS, events, mutexes, semaphores, SRW locks, condition variables and APCs |
| atlas's `loader.c`, both ISAs | An app's own DLLs, relocated, with `DllMain`, forwarders, `LoadLibrary` and `GetProcAddress`; the PEB's lists; `FreeLibrary` unloading, with detach and notifications; thread and process detach; an API set; `Ldr*`; a delay-loaded DLL; and SEH |
| atlas's `process.c`, both ISAs | Processes and anonymous pipes: `CreateProcess`, redirected output, inherited handles, exit codes |
| atlas's `pool.c`, both ISAs | The thread pool: work run many at once and waited for, cancelled, on a pool of one thread; simple callbacks and `QueueUserWorkItem`; what a callback does as it returns; timers once and repeating, and stopped; waits signalled and timed out; registered waits; cleanup groups; I/O objects on a file and a pipe, and `BindIoCompletionCallback`; timer queues |
| atlas's `fibers.c`, both ISAs | Threads made fibers and back; fibers switched between, each with its own stack, data and FLS; exceptions and a stack overflow caught on a fiber; FLS callbacks as a fiber's deleted, a thread ends and a slot's freed; a fiber that returns |
| atlas's `exceptions.c`, both ISAs | Vectored exception and continue handlers, in order, carrying on and passing; guard pages, as `VirtualQuery` and `VirtualProtect` see them; a stack overflow caught, on two threads |
| atlas's `sections.c`, both ISAs | Sections and file mappings, `VirtualQuery`, and waitable timers |
| atlas's `io.c`, both ISAs | Completion ports and overlapped I/O: events, packets, `ReadFileEx` routines |
| atlas's `pipes.c`, both ISAs | Named pipes, byte and message, pending I/O, and a pipe between two processes |
| atlas's `files.c`, `io.c`, `pipes.c`, `process.c`, `sections.c`, `registry.c`, `links.c`, `environment.c`, `security.c`, `pool.c`, `windows.c`, `desktop.c`, `ansiwin.c`, `resources.c` | Also built for 32-bit x86, where every check passes too |
| atlas's `wow64.c`, 32-bit x86, with `wowchild.c` (x86-64) beside it | WoW64: IsWow64Process(2), GetNativeSystemInfo, the environment, System32 redirected (off and on, and Sysnative), the registry's 32-bit view, a 64-bit program started that starts a 32-bit one; and the TEB through fs and the PEB; GetSystemInfo's 32-bit layout; memory below 2 GiB; the heap, the last error, TLS and critical sections; GetProcAddress; a thread; an upcall; fibers; RaiseException to a vectored handler; a frame of its own on the fs:[0] chain unwound by RtlUnwind; an access violation from atlas |
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
| 4. ntdll | the loader, heap, Rtl, the Nt stubs | `guest/`: loader, heap, sync, exceptions, environment, the thread pool; 146 exports, two of them `Nt*`/`Zw*` |
| 5. The Win32 core | kernelbase, kernel32, user32, gdi32, advapi32, shell32 | `os/`: six DLLs, 663 exports |
| 6. The runtimes | C runtime, COM, shell UI, graphics, networking, crypto, media, devices | ucrtbase, in the program (`guest/src/crt.rs`, `stdio.rs`) |
| 7. The programs Windows ships | cmd, reg, regsvr32, rundll32, msiexec, explorer, … | none |

The Win32 core, by atlas's `dll!` declarations (a stub is a `missing` export, declared but ending the process
naming itself):

| DLL | Windows' exports | atlas: host | in ntdll | stubs | declared |
| --- | ---: | ---: | ---: | ---: | ---: |
| kernel32 | 1,561 | 235 | 131 | | 23% |
| user32 | 992 | 125 | | | 13% |
| gdi32 | 545 | 20 | | 5 | 5% |
| advapi32 | 589 | 131 | | 1 | 22% |
| shell32 | 475 | 5 | | | 1% |
| version | 16 | 10 | | | 63% |
| ucrtbase | 2,560 | | 194 | | 8% |
| ntdll | 1,544 | | 35 by Windows' names, 242 of atlas's own | | 2% |

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
process, the way the page cache shares a file. COM objects atlas implements are host-side too (`os/com.rs`): an
object is a block in the program's heap of interface pointers, each to its vtable of ole32's host exports, so a
method traps into Rust like any `host` export with `this` first; what the object holds is the process's, by the
block's address. `QueryInterface`, `AddRef` and `Release` are the same three exports in every vtable, and the last
`Release` frees the block. `CoCreateInstance` makes atlas's classes (so far the shell link) and says any other isn't
registered.

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

`atlas-guest` is `no_std` Rust built by atlas's `build.rs` for `aarch64-pc-windows-msvc`, `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`,
linked with LLVM's `lld-link`, and embedded with `include_bytes!`. `core` is always built from source (see the open
questions, below). Where it can't be built, atlas says so and won't start Windows programs.

**Starting a process.** atlas maps the program and ntdll (relocating either if its base is taken), lays out the PEB,
process parameters, TEB and stack, and starts the first thread in ntdll's `AtlasProcessStart`. From there, as on
Windows, it's the loader in the program (`guest/src/loader.rs`):

- It resolves the program's imports, asking atlas to map each DLL by name (`MAP_IMAGE`). atlas's own come from their
  declarations, always; anything else is found as Windows looks, in the program's folder and then System32, and
  relocated as it's mapped, as NT's kernel does. An API set (`api-ms-win-*`) is its DLL's other name: atlas maps
  the DLL (`imports::api_set` has the map), and the loader knows it by the path it's given.
- Exports by name (binary search) and by ordinal; forwarders (`dll.name`, `dll.#ordinal`) followed to the end; the
  import table opened with `PROTECT` while it's filled in.
- Static TLS for each module with a TLS directory (its index, and this thread's block from its template), TLS
  callbacks, then each DLL's `DllMain`, dependencies first (in the order their imports were resolved), then the
  program's entry point. A `DllMain` that fails at start ends the process with STATUS_DLL_INIT_FAILED; one that
  fails in `LoadLibrary` is told DLL_PROCESS_DETACH and unloaded, as on Windows.
- The PEB's `PEB_LDR_DATA` has an `LDR_DATA_TABLE_ENTRY` for each module on its three lists, as programs walk them:
  load and memory order (the program, ntdll, kernel32, then the rest as they're loaded) and initialization order
  (ntdll, then each DLL as its `DllMain` runs), with each entry's base, entry point, size, names, flags, TLS index
  and counts.
- **References and unloading.** What the process starts with stays, as do atlas's own DLLs. A DLL loaded at run
  time counts its references (`LoadLibrary`, `GetModuleHandleEx`, and each module loaded at run time that imports
  it); `FreeLibrary` that takes the last away tells it DLL_PROCESS_DETACH and unmaps it (`UNMAP_IMAGE`), and lets
  go of what it imported, which goes too if nothing else holds it. A forwarder's DLL is pinned.
- **Ending.** A thread that ends tells each DLL DLL_THREAD_DETACH, newest first (unless it asked not to hear about
  threads with `DisableThreadLibraryCalls`), and lets go of its static TLS. `ExitProcess`, the program's entry point
  returning, or the last thread ending is ntdll's `RtlExitUserProcess`: the process's other threads end first, then
  each DLL hears DLL_PROCESS_DETACH, newest first, with a non-null `lpReserved`, and the program's TLS callbacks
  too. A thread or process ended from outside (`TerminateThread`, `TerminateProcess`) tells no one, as on Windows.
- **Delay loads**: `ResolveDelayLoadedAPI`, which MSVC's delay-load helper and Windows' own DLLs call, loads the DLL
  the first time one of its imports is called, keeps its handle where the descriptor says, and writes the export in
  the thunk; when either fails, it asks the failure hooks, and with nothing from them raises the helper's
  exception (0xC06D007E for the DLL, 0xC06D007F for the export).
- `LoadLibraryA/W`, `LoadLibraryExA/W`, `GetProcAddress`, `GetModuleHandleA/W`, `GetModuleHandleExA/W`,
  `GetModuleFileNameW`, `FreeLibrary`, `FreeLibraryAndExitThread` and `DisableThreadLibraryCalls` are kernel32 guest
  exports that forward to it. ntdll has Windows' own: `LdrLoadDll`, `LdrUnloadDll`, `LdrGetDllHandle`,
  `LdrGetProcedureAddress`, `LdrFindEntryForAddress`, `LdrResolveDelayLoadedAPI`, and DLL notifications
  (`LdrRegisterDllNotification`), which hear each DLL loaded and unloaded.

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
`WAKE_ADDRESS`, on `kernels-core`'s futexes). `TlsAlloc` slots are in the TEB, and `GetCurrentThreadId` and friends
read the TEB. The exception dispatch in progress is per thread, in the TEB.

**FLS** (`guest/src/sync.rs`): each thread's and each fiber's slots are a block the TEB's `FlsData` points at, made
when first used, and every block is on one list. A slot's callback (`FlsAlloc`'s) has each value that isn't 0 as its
thread ends (before DLLs hear DLL_THREAD_DETACH, as Windows runs them), as its fiber's deleted, and as the process
ends, for the thread that ends it; `FlsFree` gives it every thread's and fiber's value, from the list, before the
slot is free.

**Fibers** (`guest/src/fiber.rs`), all in the program. A fiber is a block with its parameter first (`GetFiberData`
reads it through the TEB's `FiberData`, as winnt.h does), the TEB's stack fields and FLS block as they were when it
last ran, and its callee-saved registers (x64's eight and xmm6 to xmm15; ARM64's x19 to x30 and d8 to d15).
`SwitchToFiber`, in assembly for each ISA, saves the running fiber's, puts the other's in the TEB, and carries on
where it left off. `CreateFiber(Ex)` gives a fiber a stack of its own, a megabyte or what it asks, with guard pages
at its bottom, so a stack overflow on a fiber is caught as on a thread; its routine returning ends the thread, with
0. `ConvertThreadToFiber(Ex)`, `ConvertFiberToThread`, `IsThreadAFiber`, and `DeleteFiber`, which runs the fiber's
FLS callbacks and frees its stack, or ends the thread if it's the fiber running.

**The thread pool** (`guest/src/pool.rs`), in the program, as Windows' ntdll has it. A pool is a queue of callbacks
and the threads that run them: a worker takes the next and runs it, and with nothing to do waits on the pool's
signal (by address, in atlas); after 20 seconds idle it ends, down to the pool's minimum. Submitting wakes an idle
worker, or starts one if none is and the pool's below its maximum (512 for the process's own). Workers are threads
like any other, started by atlas (`CREATE_THREAD`) through ntdll's thread start, so DLLs hear of them.
- **Work:** `CreateThreadpoolWork`, `SubmitThreadpoolWork`, `WaitForThreadpoolWorkCallbacks` (which can cancel what
  hasn't started) and `CloseThreadpoolWork`, which lets the object go once its callbacks have run;
  `TrySubmitThreadpoolCallback` and `QueueUserWorkItem`, once each.
- **Pools:** `CreateThreadpool`, `SetThreadpoolThreadMaximum` and `Minimum` (the minimum started at once), and
  `CloseThreadpool`; a callback environment's pool and cleanup group, as `TP_CALLBACK_ENVIRON` lays them out.
- **Timers:** one thread keeps every timer set, sleeping until the next is due by the shared data page's interrupt
  time, and queues each as it comes due, again every period if it has one. `SetThreadpoolTimer` (relative or
  absolute, or not set), `IsThreadpoolTimerSet`, waiting for and closing them.
- **Waits:** each wait has a thread of its own, waiting on its handle and on an event that says the wait's been
  changed or closed, with kernel32's `WaitForMultipleObjects`; it queues one callback with WAIT_OBJECT_0 or
  WAIT_TIMEOUT. `RegisterWaitForSingleObject` is the same, every time (or once, WT_EXECUTEONLYONCE), with
  `UnregisterWait(Ex)`.
- **As a callback returns:** an event set, a semaphore or mutex released, a critical section left, a DLL freed, as
  its instance was told; `CallbackMayRunLong` starts another worker if none is idle; and
  `DisassociateCurrentThreadFromCallback` lets those waiting for the object's callbacks stop waiting for this one.
- **Cleanup groups:** objects made with one are closed together by `CloseThreadpoolCleanupGroupMembers`, their
  callbacks run first (or cancelled).
- **I/O:** the process has one completion port for the pool, made when first wanted, and a thread waiting on it.
  `CreateThreadpoolIo` associates a handle with it, keyed by the I/O object, and each completion (a file's at once,
  a pipe's when the other end writes) queues the object's callback with its OVERLAPPED, its error and its bytes.
  `StartThreadpoolIo` and `CancelThreadpoolIo` count operations as Windows asks; `WaitForThreadpoolIoCallbacks` and
  `CloseThreadpoolIo` as for work. `BindIoCompletionCallback` is the same with the older callback, on the process's
  pool. An I/O object is never freed, since a completion may still come for it.
- **Timer queues:** `CreateTimerQueue` is a group of timers; `CreateTimerQueueTimer` is a pool timer whose callback
  is `(parameter, TRUE)`, due in milliseconds, then every period (or once, WT_EXECUTEONLYONCE), on the process's
  pool; `ChangeTimerQueueTimer`, and `DeleteTimerQueueTimer` and `DeleteTimerQueue(Ex)` waiting for callbacks (with
  INVALID_HANDLE_VALUE) or setting an event once they've run. A null queue is the default one.

ntdll calls kernel32's host exports for events, waits and completion ports by number, as a program's stub does:
`EXPORT_NUMBER` finds it by name the first time, and `sys::call6` puts a fifth and sixth argument where Windows'
convention has them (`x4` and `x5`, or the stack past the home space).

**Files** (`files.rs`, `os/fileapi.rs`): Windows paths (`C:\...`, relative to the current directory, root-relative,
`\\?\`, either slash, `.` and `..`, trailing dots and spaces dropped) onto the machine's disk, which folds case.
`CreateFileW`/`A` with every disposition, access checks, folders by backup semantics, delete-on-close, and the `NUL`,
`CON`, `CONIN$` and `CONOUT$` devices; `ReadFile` and `WriteFile` at the file's position or an `OVERLAPPED` offset;
`SetFilePointer(Ex)`, `GetFileSize(Ex)`, `SetEndOfFile`, `GetFileType`, attributes and `GetFileInformationByHandle`;
folders, delete, move, copy; `FindFirstFile`/`FindNextFile` with `*` and `?`; the current directory, full and temp
paths. Errors are Windows' own: ERROR_FILE_NOT_FOUND where the folder's there, ERROR_PATH_NOT_FOUND where it isn't.

- **Sharing.** Every file open on the disk is known by its inode, with what it was opened for (reading or running,
  writing, deleting) and its `FILE_SHARE_*`. An open for something another doesn't share, or that doesn't share what
  another is open for, is ERROR_SHARING_VIOLATION; one for attributes only isn't checked, either way. `DeleteFile`,
  `RemoveDirectory` and `MoveFileEx` (the file moved, and one it replaces) need everything that has it open to share
  deleting. Deleting is Windows 10's: the name goes at once, and what has the file open keeps reading and writing
  it until it's closed, since an open file holds its inode on the disk.
- **Links.** `CreateHardLink` gives a file (not a folder) another name; `GetFileInformationByHandle` counts them and
  gives both one file number. `CreateSymbolicLink` makes a link to a Windows path, absolute or from the link's own
  folder, of a file or (SYMBOLIC_LINK_FLAG_DIRECTORY) a folder. Opening through one reaches its target; a link to
  nothing is there, but opening it is ERROR_FILE_NOT_FOUND. `GetFileAttributes` and `FindFirstFile` see the link
  itself, a reparse point (with IO_REPARSE_TAG_SYMLINK) and a folder if it's a folder's; `DeleteFile` and
  `RemoveDirectory` take the link away, and leave what it leads to.
- **Times.** A file's last write is the disk's own; when it was made and last read are kept beside it (an extended
  attribute), set as it's made. `SetFileTime` sets any of the three, and `GetFileTime`, `GetFileAttributesEx`,
  `GetFileInformationByHandle` and `FindFirstFile` give them.

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

- **Change notification.** The registry notes each change as it's made: a key's subkey made or deleted
  (REG_NOTIFY_CHANGE_NAME), a value set or deleted (LAST_SET), its descriptor (SECURITY); a key deleted is every
  kind of change to it. After each advapi32 call, the watches those changes match have their events set, once
  each: `RegNotifyChangeKeyValue` on the key, or (with `bWatchSubtree`) anything under it, by its filter. Not
  asynchronous, it waits for the change itself.
- **The 32-bit view.** With KEY_WOW64_32KEY, `RegOpenKeyEx`, `RegCreateKeyEx` and `RegDeleteKeyEx` find the key as
  WOW64 redirects it: `HKLM\SOFTWARE\X` is `SOFTWARE\WOW6432Node\X`, but for the keys both views share (`Clients`,
  `Policies`, `Classes` but for `CLSID`, `AppID`, `Interface`, `TypeLib` and the media types, which are
  `Classes\WOW6432Node`'s, and a few of Microsoft's). A fresh machine's 32-bit view has Windows' version keys, with
  `C:\Program Files (x86)` as its program files.
- **Hives.** `RegSaveKey(Ex)` writes a key and what's under it to a new file, in atlas's format, with
  SeBackupPrivilege enabled. With SeRestorePrivilege, `RegRestoreKey` replaces a key's contents with a file's,
  `RegLoadKey` makes a file a new key under `HKEY_LOCAL_MACHINE` or `HKEY_USERS` (not saved with the machine's
  registry, but written back to its file at `RegFlushKey` and as it's unloaded), and `RegUnLoadKey` takes it away.

**The environment** (`environment.rs`, ntdll's `env.rs`): atlas makes a process's first environment as Windows does
at logon: the system `Environment` key under `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager`, then the
user's (whose `Path` goes after the system's), expanded; the session's variables (the user's folders and name, the
computer's name, `SystemRoot`), `PROCESSOR_ARCHITECTURE` for the program's ISA and `NUMBER_OF_PROCESSORS`; then any
the machine was started with (`Options::env`). It goes in the process parameters as Windows' block of UTF-16
`NAME=VALUE` strings, sorted by name, and from there the program works on it in ntdll, with no call into atlas:
`GetEnvironmentVariable`, `SetEnvironmentVariable` (a NULL value deletes; each change makes a new block on the
process heap, kept in order), `ExpandEnvironmentStrings` (names it doesn't know stay as they are),
`GetEnvironmentStrings` and `FreeEnvironmentStrings`, each in W and A. Names fold case.

**Code pages** (`ansi.rs`, `os/nlsapi.rs`). atlas is a US English Windows: its ANSI code page is Windows-1252
(`GetACP`) and its OEM one 437 (`GetOEMCP`, and the console's until `SetConsoleOutputCP` says otherwise); UTF-8,
US-ASCII and Latin-1 convert too (`IsValidCodePage`, `GetCPInfo(Ex)`). `MultiByteToWideChar` and
`WideCharToMultiByte` convert as Windows' do: NUL-terminated or counted, how much room it takes when there's none
given, ERROR_INSUFFICIENT_BUFFER when there's too little, MB_ERR_INVALID_CHARS and WC_ERR_INVALID_CHARS for UTF-8
that isn't, and a default character (the program's, or `?`) for what a code page hasn't, saying whether it was used.
Every `*A` function reads and gives back its strings in 1252, the same conversion ntdll's environment functions use
(the table is in `atlas-abi`), so a name with é in it is one name to both: files and folders, paths, the module's
own name, the command line, the registry, objects' names, accounts and SIDs, resources, locales, the clipboard's
CF_TEXT (and CF_OEMTEXT in 437), and version information, whose `VerQueryValueA` converts a string value into the
room after the block that `GetFileVersionInfoSizeA` asked for. `WriteConsoleA` shows its bytes in the console's code
page; `WriteFile` to the console passes them through, so a program that writes UTF-8 is shown as it means.

**The C runtime** (`os/ucrtbase.rs`, `guest/src/crt.rs`, `stdio.rs`). ucrtbase.dll is part of Windows 10, so atlas
has its own, and every export runs in the program (as ntdll code): programs reach it by name or by its API sets
(`api-ms-win-crt-*`), as MSVC's `/MD` and MinGW's UCRT programs do. It has the heap (`malloc` and the rest on
ntdll's), memory and strings narrow and wide, character classes, `strtol`, `strtod` and the rest, `qsort`, `rand`
(Windows' own sequence), `errno` (in atlas-guest's room in the TEB), and startup and exit: the arguments split as the
C runtime splits them, the environment, `_initterm`, onexit tables, `atexit`, `exit`, `abort` and `signal`. Its stdio
has the three standard streams and `fopen`'s, over kernel32's files (called by number), in text mode unless binary;
writes aren't buffered, reads are. Its `printf` engine, behind `__stdio_common_v*printf`, has every conversion but
`%a` and `%n`, every flag and size, narrow and wide. A MinGW-built x86-64 C program runs as it is.

**ANSI windows.** Inside win32k every message's text is UTF-16. A class registered with `RegisterClass(Ex)A` is ANSI,
and so are its windows (`IsWindowUnicode` FALSE), until `SetWindowLongPtrW` gives one a `WndProc` (and
`SetWindowLongPtrA` makes one ANSI). A message to an ANSI window's `WndProc` is converted on the way in: WM_SETTEXT's
text, WM_NCCREATE's and WM_CREATE's CREATESTRUCT names, and WM_CHAR's (and the other character messages')
character; WM_GETTEXT's buffer is the procedure's to fill in ANSI and is converted back into the caller's. The `*A`
functions convert the other way as a message comes in: `SendMessageA`, `PostMessageA`, `DispatchMessageA`,
`DefWindowProcA`, `SetWindowTextA` and `GetWindowTextA`; `GetMessageA` and `PeekMessageA` give characters in ANSI;
`CreateWindowExA` takes its names in ANSI. Converted text goes on the calling thread's stack, below where it trapped,
with the stack pointer lowered while the message is out (`Task::with_scratch`), so a message sent to another thread
reads it while the sender waits.

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
`AdjustWindowRectEx`, `GetWindowRect`, `ClientToScreen`, `ScreenToClient` and `WindowFromPoint`. A disabled
window (`EnableWindow`) and the windows in it take no clicks. Painting a window marks the children it covers to paint
again, since they're drawn on the same pixels, and only windows whose every ancestor is shown are painted.
`DefWindowProc`'s WM_PAINT erases first, as `BeginPaint` does. A window's system menu (`GetSystemMenu`) is its
commands' states only, with no menu to show yet; a command greyed there (`EnableMenuItem`), such as SC_CLOSE, isn't
carried out by `DefWindowProc`, though the desktop's close button still looks the same.

**Dialogs and controls** (`win32k/dialog.rs`, `win32k/controls.rs`). The system classes are win32k's: `#32770` (atom
0x8002), Button, Static and Edit, whose window procedures are host Rust. A window holds such a procedure as a value no
program address can be; a program asking for it (`GetClassInfo`, `GetWindowLongPtr`, `GetClassLongPtr`) gets the
address of the user32 export that runs it (`DefDlgProcW`, and `AtlasButtonWndProc`, `AtlasStaticWndProc` and
`AtlasEditWndProc`, which Windows doesn't export), so `CallWindowProc` and a class of the program's own registered
with it reach the same code. A dialog comes from a template, `DLGTEMPLATE` or `DLGTEMPLATEEX`, from a resource
(`CreateDialogParam`, `DialogBoxParam`) or from memory (the `Indirect` forms). Dialog units are a quarter of the
font's average character's width and an eighth of its height; with the one font, that's its 6 by 13 cell, which
`MapDialogRect` and `GetDialogBaseUnits` say too. The dialog is made, then `WM_SETFONT`, then each control (with its
creation data as `WM_CREATE`'s parameter, and the dialog failing if one can't be made, unless `DS_NOFAILCREATE`),
then `WM_INITDIALOG`, whose TRUE puts the focus on the first control that takes Tab. Its procedure, its result and
`DWLP_USER` are in its extra bytes where Windows keeps them (`DLGWINDOWEXTRA`: 30 bytes for a 32-bit program, 48 for
a 64-bit one), so `GetWindowLongPtr` and `SetWindowLongPtr` reach them; `GetWindowLong` and `SetWindowLong` read
four bytes of extra bytes, the Ptr forms a pointer's width. `DefDlgProc` calls the procedure and, where it returns
FALSE, does the rest: the dialog's face, WM_CLOSE as IDCANCEL, the focus kept and given back as the dialog is
deactivated and activated, `DM_GETDEFID` and `DM_SETDEFID` (which restyle the push buttons), `WM_NEXTDLGCTL`.
`DialogBox` disables the owner, runs the dialog's own message loop until `EndDialog` (which hides it), then enables
the owner, destroys the dialog and activates the owner; a WM_QUIT ends the loop and is posted again.
`IsDialogMessage` asks the control with the focus what it wants (`WM_GETDLGCODE`) and gives Tab and Shift+Tab (in
creation order, into children that are `WS_EX_CONTROLPARENT`), Enter (the focused push button, the default one, or
IDOK, unless it's disabled), Escape (IDCANCEL) and the arrows (within a group) their meaning in a dialog, translating
and dispatching the rest. The controls paint into the top-level window's pixels in the colours their parent's
`WM_CTLCOLOR*` gives (its brush, a hollow one leaving what's under it, and the DC's text colour, background and
mode as it left them): push buttons (default and focused), check boxes and 3-state ones, radio buttons (an
automatic one unchecking the others in its group), group boxes; statics' text (wrapped at its width, `&` mnemonics
taken out), rectangles, frames and etched lines; one-line and multi-line edits with a caret, selection, typing,
Backspace and Delete, the arrows, Home and End, a password character, read-only, number-only and upper- or
lower-case edits, a limit, and the `EM_*` messages programs use most. Their parents hear `BN_CLICKED`, `STN_CLICKED`,
`EN_CHANGE`, `EN_UPDATE`, `EN_SETFOCUS`, `EN_KILLFOCUS` and `EN_MAXTEXT`. A static's icon or bitmap (`SS_ICON`,
`SS_BITMAP`: from `STM_SETIMAGE`, or loaded from its module by the resource ID its template names) is drawn at its
corner or in its middle (`SS_CENTERIMAGE`), and a `BS_ICON` or `BS_BITMAP` button's picture in place of its text; an
owner-drawn button is only its face.

**The shell** (`os/shellapi.rs`, for shell32). Special folders (`SHGetFolderPath`, `SHGetSpecialFolderPath`,
`SHGetSpecialFolderLocation`, `SHGetFolderLocation`) are the profile's, by CSIDL: `C:\Users\carbon` with its
AppData and start menu, `C:\ProgramData` for everyone's, `C:\Users\Public`, and Windows' own, Program Files and
System being the 32-bit ones for a 32-bit program; one asked for is made on the disk if it isn't there, as a profile
has them all (a fresh disk has the common ones, and the registry's Shell Folders say the same). An item ID list is one
item holding a full path (the desktop's is empty, and a virtual folder's has no path), allocated from the program's
heap through ntdll's task allocator so `CoTaskMemFree` and `ILFree` free it. `SHFileOperation` copies, moves, renames
and deletes files and whole folders, with wildcards, into a folder (made if it isn't there) or onto names, as its
flags say. `ShellExecute(Ex)` runs a program through `CreateProcess` (`runas` too, as there's one user), giving the
process with `SEE_MASK_NOCLOSEPROCESS`; documents, folders and URLs need associations and Explorer, so it says there's
no association, as Windows does for what nothing opens. `SHBrowseForFolder` is a dialog of shell32's own on the
dialog manager: the caller's text, the folder's path to type or have set (`BFFM_SETSELECTION`, which its callback
hears as `BFFM_SELCHANGED`), a status line, and OK, which takes only a folder that's there; there's no tree of folders
yet. `SHGetFileInfo` gives a file's display and type names, attributes, the system image lists and a program's type,
the icons being placeholders.

**Menus** (`win32k/menu.rs`, `os/menuapi.rs`): menu objects, built (`CreateMenu`, `CreatePopupMenu`, `AppendMenu`,
`InsertMenu`, `InsertMenuItem`, `ModifyMenu`, `SetMenuItemInfo`), read back (`GetMenuItemCount`, `GetMenuItemID`,
`GetSubMenu`, `GetMenuState`, `GetMenuString`, `GetMenuItemInfo`), changed (`CheckMenuItem`, `CheckMenuRadioItem`,
`EnableMenuItem`, the default item, `RemoveMenu`, `DeleteMenu`, `DestroyMenu` with its submenus) by position or by
command (searching submenus, as Windows does), and loaded from `MENU` and `MENUEX` resources. A window keeps its menu
(`SetMenu`, `GetMenu`); menu bars aren't drawn yet, a frame being the desktop's. `TrackPopupMenu(Ex)` sends
`WM_INITMENUPOPUP` and shows the menu as a popup of the desktop's that grabs the keyboard and pointer, placed against
the point as its flags align it and kept on the screen by the desktop's positioner, then runs the menu loop on the
calling thread: the pointer lights items up and opens submenus as popups of their own, a click chooses, the arrows
move (past separators) and open and close submenus, Enter chooses, Escape closes a level, an item's `&` letter
chooses it, and a press outside dismisses them all. It gives the item's ID (`TPM_RETURNCMD`) or posts `WM_COMMAND`
for it. Items are drawn with their text, accelerator, check or radio dot, a submenu's arrow, and greyed when they're
disabled.

**What larger programs call as they start**, found by scanning a real client program's imports (`carbon atlas
imports`) and written so it loads and starts. In the guest: `HeapReAlloc` (in place when the block has room, else
moved; zeroing what it grows by; `HEAP_REALLOC_IN_PLACE_ONLY`), `HeapSize` (the size asked for, kept in each block's
header), `HeapValidate`, `HeapLock`, `HeapWalk` (no blocks listed), `GetProcessHeaps` and the heap's information;
`EncodePointer` and `DecodePointer` (XOR with a per-process secret, then a rotation); `InitOnceBeginInitialize` and
`InitOnceComplete` (the state in the INIT_ONCE's low two bits and the context above them, as Windows keeps it, which
`InitOnceExecuteOnce` now shares); interlocked singly linked lists; `IsDebuggerPresent` (the PEB's byte), `DebugBreak`
(a breakpoint exception) and `UnhandledExceptionFilter` (the program's filter, else `EXCEPTION_EXECUTE_HANDLER`); and
oleaut32's BSTRs and `VariantInit`, `VariantClear` and `VariantCopy` (a BSTR freed or copied, an interface released or
added to). In the host: `CompareString` (word sort passing over hyphens and apostrophes, case second and lowercase
first, `NORM_IGNORECASE`, `NORM_IGNORESYMBOLS`, `SORT_DIGITSASNUMBERS`), `LCMapString` (case one character for one,
byte reversal, and sort keys in the same order) and `GetStringType`'s three classes; `GetDateFormat` and
`GetTimeFormat`, in English (United States), with pictures and their flags; `GetDiskFreeSpaceEx`, `GetDriveType`,
`FindFirstFileEx`; the console's modes, `ReadConsole`, `SetStdHandle`; `OpenThread`, thread priorities, affinity
masks and the session; psapi's module paths, module information and memory counters (as kernel32's `K32` functions
too); `IsBadWritePtr` and `IsBadReadPtr` from the process's regions; `OutputDebugString` into the trace; user32's one
monitor (`MonitorFrom*`, `GetMonitorInfo`, `EnumDisplayMonitors`), the window station and desktop
(`GetUserObjectInformation`), `MapWindowPoints`, `RedrawWindow`, and `MsgWaitForMultipleObjects(Ex)`, a wait that also
ends when a message comes or a timer is due; gdi32's fonts from memory and OpenGL's pixel formats (accepted, with
nothing to draw yet); advapi32's event log sources and `RtlGenRandom`; bcrypt's `BCryptGenRandom`, from the host's
random bytes; shell32's `CommandLineToArgvW` (Windows' quoting rules), `SHGetKnownFolderPath` (the known folders'
GUIDs, as their CSIDLs' paths) and `IsUserAnAdmin`; and Winsock's start (`WSAStartup`, `WSACleanup`), byte order,
last error and `__WSAFDIsSet`, at Windows' ordinals. Blocks over 1 MiB on the heap have pages of their own, given back
when they're freed, so a 32-bit program that unpacks big files doesn't run out of address space; and a file on the
disk reads and writes as much as it's asked for at once (up to 1 GiB), as Windows' do.

**Winsock** (`sockets.rs`, `os/ws2_32.rs`) is a thin layer over the machine's network, [`net`](../../net/README.md). A
socket is a file, as AFD makes it (`Node::Socket`), so it closes with `CloseHandle`, joins a completion port and takes
`SetFileCompletionNotificationModes` as any file does. `net` never waits; a blocking call tries, and if it would
block, waits on its thread's waiter, which a watch on the socket wakes as its readiness changes. `FIONBIO` makes it
WSAEWOULDBLOCK instead; SO_RCVTIMEO and SO_SNDTIMEO end the wait with WSAETIMEDOUT. An overlapped call (`WSARecv`,
`WSASend`, their `From`/`To` forms, `ConnectEx`, `AcceptEx`) is tried at once, and if it would wait, queued on its
socket for the process's network thread: made the first time something's queued (started at ws2_32's
`AtlasNetPump`), it wakes with any of its sockets' readiness, finishes what it can, and writes the results into the
program's buffers and OVERLAPPED, as only a thread of the process can, then sets the event, posts the port's packet
or queues the routine. One done at once is told the same way, but for the port with
FILE_SKIP_COMPLETION_PORT_ON_SUCCESS; one that fails at once is told only by its return. `closesocket` ends what's
under way with WSA_OPERATION_ABORTED. `select` and `WSAPoll` wait on any number of sockets; since `net` says a host
socket is readable until a read finds nothing, a socket said readable is checked with a byte peeked. `WSAIoctl`
gives the extension functions (`ConnectEx`, `AcceptEx`, `GetAcceptExSockaddrs`, `DisconnectEx`: exports of
ws2_32's, here, rather than mswsock's), the base handle, the address list and keep-alives. `getaddrinfo` and
`GetAddrInfoW` ask the machine's resolver (`Network::lookup`: its own name and `localhost` it answers, the rest the
host's way), with services by number or by name, each answer in one block from the heap. `net`'s errors are WSA
codes; a socket's are NTSTATUS in its OVERLAPPED, as `0xE000xxxx` with the WSA code, which `ports::error` reads back.
Not yet: `WSAEventSelect`, `WSAAsyncSelect`, `gethostbyname` and the other old lookups, IPv6-only sockets'
options, `TransmitFile`, Unix sockets, and the `iphlpapi` and `dnsapi` calls.

**Certificates** (`os/crypt32.rs`): contexts are in the program's memory as CryptoAPI gives them, CERT_CONTEXT with
its DER and a CERT_INFO read from it (version, serial, algorithms, issuer and subject, validity, the public key; not
the extensions yet). Stores are atlas's own, in memory (a system store opens empty: the host's roots aren't
listed). `CertGetCertificateChain` asks the host whether it trusts the certificate with whatever the stores hold
(`Host::check_certificates`: Security.framework on macOS, CryptoAPI on Windows, webpki on Linux), for the usage the
chain parameters ask (server or client authentication), and gives a chain context of the certificates it could link
from the leaf, its CERT_TRUST_STATUS the host's verdict. `CertVerifyCertificateChainPolicy`, which checks names, is
to come.

**Shell links** (`os/com.rs`): `CLSID_ShellLink`'s objects, with `IShellLinkW`, `IShellLinkA` and `IPersistFile`. One
keeps a target, arguments, working folder, description, relative path, icon and its index, show command and hot key,
and gives the target as a path, a WIN32_FIND_DATA and an item ID list. `IPersistFile` saves it as a `.lnk` file in
the Shell Link Binary File Format ([MS-SHLLINK]): the header (with the target's attributes and size), the target as
`LinkInfo`'s local path in ANSI and UTF-16, and the strings in UTF-16; and loads one back, from `LinkInfo` in either
form. `Resolve` takes the target as it is.

comctl32's progress bar and list view are win32k's too (`win32k/comctl.rs`), their classes there from the start as if
comctl32 had registered them as it loaded, and their procedures exported from comctl32 (`AtlasProgressWndProc`,
`AtlasListViewWndProc`). A progress bar has its range (16- and 32-bit), position, delta, step (wrapping past the top
as comctl32's does), bar and background colours, and error and paused states, drawn smooth, across or up; a marquee
is a block moved along by each `PBM_SETMARQUEE` and step. A list view keeps its columns (with their width, text and
alignment; `LVSCW_AUTOSIZE` and `AUTOSIZE_USEHEADER`) and its items (their text in each column, state, image and
`lParam`), in the W and A forms of the `LVM_*` messages programs use most: inserting, setting and getting items and
their text, deleting, `LVM_SETITEMSTATE` and `LVM_GETNEXTITEM`, single selection, scrolling (`LVM_ENSUREVISIBLE`,
`LVM_SCROLL`, the wheel), the arrows and Home, End, Page Up and Down, colours, extended styles and image lists. It's
drawn as a report, with a header unless `LVS_NOCOLUMNHEADER`; its icon, small icon and list views are drawn as one
column of the same rows for now. Its parent hears `NM_CLICK`, `NM_DBLCLK`, `NM_RCLICK` and `LVN_ITEMCHANGED`
through `WM_NOTIFY`. Image lists keep their images: a bitmap cut into them (`ImageList_Add`, transparent where a mask
bitmap is white; `AddMasked`, where a colour is), or an icon at their size (`ReplaceIcon`); `ImageList_Draw` blends
one into a DC. A list view doesn't draw its items' images yet.

GDI draws host-side, into each top-level window's pixels on the desktop: DCs from `GetDC` and `BeginPaint`, clipped
to the client area or what needs painting; stock and made brushes and pens; `FillRect`, `FrameRect`, `Rectangle`,
`MoveToEx`/`LineTo`, `SetPixel`/`GetPixel`, `TextOutW` and `DrawTextW` in the one font (5 by 7 in a 6 by 13
cell). Bitmaps are objects with their pixels (`LoadBitmap`, `LoadImage`, `CreateCompatibleBitmap`, `CreateBitmap`,
`GetObject`'s BITMAP), and a memory DC (`CreateCompatibleDC`) draws everything above into the one selected into it.
`BitBlt` and `StretchBlt` copy between any two DCs, windows' or memory ones (scaled to the nearest pixel), with
`SRCCOPY`, `SRCPAINT`, `SRCAND`, `SRCINVERT`, `NOTSRCCOPY`, `DSTINVERT`, `PATCOPY`, `BLACKNESS` and `WHITENESS`.
Icons and cursors are objects too, their pixels with alpha (`win32k/image.rs`): from a module's `ICON` and `CURSOR`
groups or an `.ico` or `.cur` file, the image nearest the size asked for (larger before smaller, then most colours)
and scaled to it, transparent by a 32-bit image's alpha or by its mask; `DrawIconEx` and `DrawIcon` blend them, and a
window's class cursor or `SetCursor`'s is shown on the desktop as its picture, a system one (`LoadCursor` with no
module) as the desktop's shape. Device-independent bitmaps are read in 1, 4, 8, 16, 24 and 32 bits, with palettes or
bit fields, bottom-up or top-down; RLE and PNG ones (Vista's large icon images) aren't yet, and an icon group whose
only image is PNG gives a blank icon its size. The desktop is a screen in memory (1280 by 800, or what `atlas::Options` gives), shown on a Mac with
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
- **Vectored handlers** (`AddVectoredExceptionHandler`) are asked first, before any frame's, those added `first`
  before the rest; one that says to carry on (-1) ends the search. Vectored continue handlers
  (`AddVectoredContinueHandler`) then hear every exception the program carries on from, however it was handled.
- Unhandled, it says which exception and where, and the process ends with its code.

**Guard pages and the stack.** A page committed or protected with PAGE_GUARD (`VirtualAlloc`, `VirtualProtect`, and
`VirtualQuery` saying so) is mapped with no access; its first touch takes the guard away from that page and raises
STATUS_GUARD_PAGE_VIOLATION. Every thread's stack is a megabyte committed as it's touched, whose bottom 16 KiB are
guard pages (the TEB's `DeallocationStack` says where the stack starts): touching them raises STATUS_STACK_OVERFLOW,
with the guard taken from all of them, so the handler has the room to run in, and the TEB's `StackLimit` moved down.
A second overflow has none left, and ends the process, as on Windows. exec's x86-64 translation makes a `push` and a
`call` store before they move `rsp`, so a fault on the stack is precise and the unwinder sees the frame as it was.

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
  wait.
- **Memory and sections:** a `WriteFile` to a mapped file seen by its views (one cache for both); `SEC_RESERVE`;
  image sections; `SetThreadStackGuarantee`, and stack sizes from the PE header; `VirtualAlloc2`'s placeholders; write
  watch; large pages, refused politely; one namespace for sections' and other objects' names.
- **Files and I/O:** junctions and other reparse points (`FSCTL_GET_REPARSE_POINT` and the rest), opening a link
  itself (FILE_FLAG_OPEN_REPARSE_POINT), and `GetFinalPathNameByHandle` through a link; `ReplaceFile`, and `MoveFileEx` until reboot; change notification (`ReadDirectoryChangesW`); other drive
  letters, `GetVolumeInformation` and free space; overlapped pipe writes that wait for room,
  `GetNamedPipeHandleState`, `CancelSynchronousIo`; mailslots; `DeviceIoControl` for volumes, the console, and AFD
  for sockets **[net]**.
- **The registry:** Windows' own hive format (`regf`), so `RegLoadKey` takes a hive from a real Windows (a default
  user's `NTUSER.DAT`); the 32-bit view of `HKCU\Software\Classes`, and 32-bit programs in it by default; a key's
  notification ending as its handle's closed; saving more often than at a flush and the machine's end; transactions,
  refused cleanly.
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
  besides US; menus' rest (menu bars drawn in a window's frame and their keyboard, accelerators, the system menu
  shown, owner-drawn and bitmap items, `WM_MENUSELECT` and the other notifications);
  dialogs' rest (menus in templates, `DS_SETFOREGROUND` and the other `DS_*` styles, owner-drawn buttons' WM_DRAWITEM,
  icons and bitmaps in statics and buttons, keyboard mnemonics); the standard controls still to come (LISTBOX,
  COMBOBOX, SCROLLBAR, and an edit's scrolling, clipboard and undo); comctl32's others (tree view, tabs, trackbar,
  up-down, status bar, toolbar, tooltips, the header as a control of its own) and the list view's rest (its icon
  views, editing labels, sorting, `LVN_GETDISPINFO`'s callback items, owner data);
  scroll bars and carets; icons' rest (`GetIconInfo`, `CreateIconIndirect`, `CopyImage`, PNG images, the system's own
  icons drawn, a window's icon in its frame); hooks
  (`SetWindowsHookEx`); window stations and desktops as objects (there's one of each, by fixed handles, for now);
  `MsgWaitForMultipleObjects`' wake mask, told apart by kind of message; windows across processes (`SendMessage`, `EnumWindows`, `WindowFromPoint`);
  per-monitor DPI awareness; IME, with imm32.
- **NtGdi:** bitmaps' rest (`CreateDIBSection` on shared pages, `StretchDIBits`, `SetDIBitsToDevice`, `GetDIBits`,
  `AlphaBlend`, `TransparentBlt`, monochrome bitmaps' colours); regions; ellipses, polygons and paths; mapping modes; real fonts with metrics and the
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
- **The loader:** side-by-side manifests and activation contexts; a loader lock held across `DllMain`, as Windows
  has, so two threads loading at once wait for each other; dependency cycles between DLLs loaded at run time, which
  stay loaded; `LoadLibraryEx`'s flags (`LOAD_LIBRARY_AS_DATAFILE`, search paths) and `SetDllDirectory`;
  `GetModuleHandle` by an API set's name; a schema of API sets in the PEB, for programs that read it themselves.
- The thread pool's rest: callback priorities, WT_EXECUTEINTIMERTHREAD and the other legacy flags, and the `Tp*`
  exports by Windows' names; C++ exceptions' support
  (`__CxxFrameHandler3`/`4`, `_CxxThrowException`) where no vcruntime is brought; `Etw*` that succeed and do nothing.

### 5. The Win32 core

- **kernel32:** process lists (toolhelp) and psapi's enumerations (`EnumProcesses`, `EnumProcessModules`); the
  console in full (input records, screen buffers, cursor and colours, code pages, `AllocConsole`, virtual terminal
  sequences); strings and NLS (code pages beyond 1252, 437 and UTF-8, the double-byte ones too; sorting as Windows'
  tables do, accented letters with their plain ones; `GetLocaleInfoEx`, number formats, other locales' dates); resources' rest (`UpdateResource`, `LOAD_LIBRARY_AS_DATAFILE`,
  message tables for `FormatMessage`); job objects; `SetThreadExecutionState`;
  the long tail, as the import scanner and the `missing` stubs say.
- **advapi32:** services; the event log kept and read (its sources are taken and their reports let go, for now);
  CryptoAPI's legacy face over bcrypt; ETW registration that succeeds.
- **user32 and gdi32:** ANSI windows' rest: `GetWindowLongPtrA`'s and `GetClassLongPtrA`'s `WndProc` for a Unicode
  window (and the W ones' for an ANSI one) as a handle `CallWindowProc` converts through, as Windows gives, and
  controls' messages with text (`EM_REPLACESEL` from an ANSI caller, `LB_*`, `CB_*`); menu, icon, cursor and
  accelerator resources;
  `SystemParametersInfo` and the system colours.
- **The shell's core** (shell32, shlwapi, shcore): known folders by GUID (`SHGetKnownFolderPath`); `ShellExecute(Ex)`
  by association (documents, folders, URLs), with Explorer; `IFileOperation`, and `SHFileOperation`'s progress,
  confirmations and recycle bin; shell links' item ID lists as Windows makes them (a link with only those loads with
  no target), and resolving a link whose target moved; a tree of
  folders in `SHBrowseForFolder`; icons and the system image list drawn; `CommandLineToArgvW`; shlwapi's path, string
  and registry helpers; shcore's DPI and stream helpers.

### 6. The runtimes

OS DLLs are atlas's; redistributables a program ships (vcruntime140, msvcp140, the .NET runtime) are the program's,
running on atlas. Where a group's real surface is COM (Direct3D, DirectWrite, WASAPI, WIC, the shell's objects), each
interface is the to-do, not each export.

| Group | DLLs | To do |
| --- | --- | --- |
| C runtime | ucrtbase, msvcrt | **[guest]** ucrtbase's rest, as a MinGW C++ program asks: `setjmp`/`longjmp`, `_beginthreadex`, low-level I/O (`_read`, `_write`, `_fstat64`), wide stdio, `strftime`, the multibyte conversions, math; then msvcrt.dll for older MinGW programs |
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

- **32-bit x86 (WoW64)** **[exec]** **[guest]**: most installers are 32-bit. In
  order, each phase ending with its tests passing:
  1. [x] **exec runs 32-bit x86** on hvf: the translator's 32-bit mode, checked by exec's conformance suite. kvm
     and winkvm have a 32-bit code segment in the shim already; their conformance cases still need running there.
  2. [x] **The i686 ntdll**, built and embedded beside the others, though nothing runs it until phases 3 and 4 do:
     `atlas-abi` has every layout with pointers in it twice (`teb32`, `peb32`, `ldr32`, `params32`, `record32`, and
     x86's 0x2cc-byte `CONTEXT`), the guest gets its own ISA's; every export takes pointer-sized arguments as
     `usize` (stdcall would pass a `u64` as two slots), and pointer-sized memory as `pe::ptr_at`/`put_ptr` (PE32's
     thunks, TLS directory and image base too); SRW locks, condition variables, TLS and FLS slots are a pointer
     wide; printf reads x86's `va_list` (4-byte slots, 8 for `long long` and doubles). The TEB is `fs:[0x18]`;
     atlas's calls are `int 0x2e` with the arguments pushed above a dummy return address, as a stdcall stub has
     them; fibers switch in i686 assembly, each with its own SEH chain. SEH is x86's (`seh32.rs`): the chain from
     `fs:[0]`, cdecl handlers, `RtlUnwind` calling and unlinking frames above its target, and `RtlRaiseException` and
     `RtlUnwind` as stubs that capture their caller's registers. `_alldiv`, `_aulldiv`, `_allrem` and `_aullrem` wrap
     compiler_builtins'. Still to do there: a nested exception's own registration (`ExceptionNestedException` is
     taken as handlers return it, not detected), and `_except_handler3`/`4` for the C runtime (phase 6).
  3. [x] **32-bit processes**, run on hvf: `wow64.c` passes. `images::layout(isa)` says where things go: a 32-bit
     process's allocations from 64 KiB to 1.5 GiB, atlas's page at 0x6fff0000 and the system DLLs from 0x70000000,
     all below 2 GiB, as WoW64 has them. `abi.rs` gives each process its own ISA's TEB, PEB, process parameters and
     exception record offsets and pointer width, which start-up, threads (`fs` at a TEB whose SEH chain starts at
     -1), `MAP_IMAGE`, `GetSystemInfo`'s 36-byte SYSTEM_INFO and exceptions use: x86's are an EXCEPTION_RECORD32 and
     a 0x2cc-byte CONTEXT, with FloatSave and ExtendedRegisters both, handed to the dispatcher stdcall's way, and
     `NtContinue` takes what the CONTEXT's flags say it has. A 32-bit call's arguments are read off its stack; -1 to
     -16 are widened as negative numbers, as WoW64 widens a handle, so pseudo-handles and INVALID_HANDLE_VALUE are
     what 64-bit code expects, and its result is cut to 32 bits. PE32 images were already read and relocated.
  4. [x] **System DLLs and host exports for i686**: `images/make.rs` makes PE32 images whose host stubs are
     `mov eax, nr; int 0x2e; ret 4*n`, popping the arguments stdcall's way (the `(n)` in each `host` declaration, each
     checked against Windows' signature), and `ret` alone for the cdecl ones (ucrtbase's, `wsprintf`, `DbgPrint`).
     The four that take a 64-bit value by value (`SetFilePointerEx`, `VerifyVersionInfo`, `VerSetConditionMask`,
     `WindowFromPoint`) read two slots for it (`images::x86_wide`), and `GetTickCount64` and `VerSetConditionMask`
     return their high half in edx. Every structure a host export reads or writes with a pointer, handle or SIZE_T
     in it follows the process's `Abi`: OVERLAPPED (written from another thread too, by the process's ISA) and
     OVERLAPPED_ENTRY, completion keys, out-handles (keys, pipes, tokens, `DuplicateHandle`), STARTUPINFO and
     PROCESS_INFORMATION, SECURITY_ATTRIBUTES, absolute security descriptors and the PSID and PACL outs, token
     information (SID_AND_ATTRIBUTES, TOKEN_GROUPS and the pointer forms), MEMORY_BASIC_INFORMATION, MEMORYSTATUS,
     the logical processor records, WNDCLASS(EX), MSG, CREATESTRUCT (and its ANSI copy), PAINTSTRUCT, INPUT,
     FLASHWINFO, `LoadString`'s and `VerQueryValue`'s pointers, `GetFullPathName`'s file part, and
     `NtQuerySystemInformation`'s basic information and process list. Its handle lists and the kernel's modules
     answer that the class isn't there for a 32-bit program, for now. A 32-bit process has the shared data page
     too, saying the machine is AMD64, as x64 Windows' WoW64 does.
  5. [x] **WoW64 as programs see it**: a 32-bit program's System32 is SysWOW64 (`Task::resolve`), but for the
     folders both kinds share (`catroot`, `catroot2`, `driverstore`, `drivers\etc`, `logfiles`, `spool`) and while
     its thread has turned that off (`Wow64DisableWow64FsRedirection`, `Revert`, `Enable`); `Sysnative` is System32,
     always. SysWOW64 holds atlas's PE32 DLLs, System32 the 64-bit ones, and the loader looks in SysWOW64 for a
     32-bit program's. The registry's 32-bit view is a 32-bit program's unless it asks for `KEY_WOW64_64KEY` (and
     `RegGetValue` takes `RRF_SUBKEY_WOW64*KEY`). `IsWow64Process` says yes for a 32-bit process, `IsWow64Process2`
     I386 under AMD64, and `GetNativeSystemInfo` x64, as x64 Windows' WoW64 does (on an ARM64 Mac too: atlas is an
     x64 Windows to its programs). As each process starts, the variables that say what kind it is are set for it,
     whatever its parent's said: PROCESSOR_ARCHITECTURE (x86, with PROCESSOR_ARCHITEW6432 AMD64), ProgramFiles and
     CommonProgramFiles (the (x86) folders for a 32-bit program). 32-bit and 64-bit programs start each other.
     `carbon run` knows a 32-bit `.exe` (platform `windows/386`), and gives it x86's memory ordering.
  6. [ ] **Every test program for i686**, passing; then real installers. Every test program but `sysinfo.c` (whose
     checks are 64-bit offsets; `wow64.c` checks the same calls at 32 bits) builds for i686 and passes every check:
     30 programs, 1,076 checks, with the loader's own DLLs, thread-local variables (`__tls_array`), `__try` through
     `_except_handler3` (now in the i686 ntdll, with `_local_unwind2`), and the C runtime. `build.sh` makes each
     program's import libraries from what its object imports, `__imp_` or not. `installer.c` checks what installers
     call first, on all three ISAs: `wsprintf` is C-variadic, so each ISA's is a few lines of the guest's assembly
     that make a `va_list` of its registers and stack and call `wvsprintf` with it, which the printf engine formats.
     A real 32-bit installer now starts, unpacks its compressed data, runs its start-up script (the registry, files,
     `.onInit`'s checks), and shows its main dialog with its first page (a child dialog of its own class, made from
     what `GetClassInfo` gave for `#32770`): text, a group box, an edit with the install folder, the disk space, and
     its buttons, which work. Its Install button takes it to its progress page (a progress bar and a details list,
     comctl32's), where it extracts its files with each one shown, the bar filling as it goes, finds the start menu
     and desktop folders, makes its shortcuts there (shell links saved as `.lnk` files), and finishes on its own, its
     dialog ended and its temporary files removed. Still to do there, in order:
     - **The rest:** `MessageBoxIndirect`, `SendMessageTimeout`, `SystemParametersInfo`, `GetMessagePos`,
       `ExitWindowsEx`, which its other paths reach (an error, a reboot, its uninstaller).
- **ANSI and code pages:** each `*A` function still to come is the `*W` with 1252 strings, as those there are.
- **Manifests:** side-by-side assemblies and activation contexts decide which comctl32 a program gets.
- **API sets:** `ext-ms-*` sets, and the `api-ms-win-*` the map doesn't know yet, as programs ask.

### What real programs will ask for next

Installers, winget and large desktop apps go past the layers' first items, in roughly this order:

1. **What stops them starting:** `__unDName` (checked against `llvm-undname`) and the C++ runtime's internals;
   `reg.exe`, `regsvr32.exe`, `rundll32.exe` and `cmd.exe`; then `conhost`, `taskkill`, `icacls`, `schtasks`,
   PowerShell for installer scripts, `explorer.exe`.
2. **What ordinary programs use:** menus; real fonts and the rest of GDI, with `DrawTextExW`, `ScrollWindowEx` and
   Uniscribe; real icons and cursors; services; restricted tokens, job objects with limits, and desktops and window
   stations (a browser engine's sandbox); `msi.dll` with compound files written.
3. **What's faked underneath:** change notification; junctions; `MoveFileEx` until reboot; transactions; windows across processes and `SetWindowsHookExW`;
   the environment from the registry so what an installer sets reaches new processes; process
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
  `gdi.c`, `com.c`, `winsock.c`, `crypto.c`.
- **The long tail:** `carbon atlas imports` ranks what folders of real programs lack; next is counting the `missing`
  stubs and `GetProcAddress` names programs reach as they run.
- **Checkpoints**, each run headless and screenshotted: a console tool, a plain Win32 program, a dialog program with
  the common controls, an installer built from source, a networked program, a Direct3D program, a browser
  engine's sandboxed processes.

The kernel's areas come first and are done. Then, each step ending with its tests passing on both ISAs: ntdll's
foundation (the `Nt*` exports, `Rtl*`; the loader's lists, unloading, delay loads, API sets, FLS, fibers and the
thread pool are done); 32-bit, while the layers are thin; the Win32 core's breadth (NLS, the console, advapi32's
services, the shell's core); the window kernel's breadth (bitmaps, fonts, regions, menus, dialogs, controls, icons,
`MsgWaitForMultipleObjects`); the services; the runtimes, group by group; and the programs Windows ships.

### Open questions

- **Win32 over Nt, or Win32 in host Rust?** On Windows, kernelbase's `CreateFileW` runs in the program and calls
  `NtCreateFile`; atlas implements `CreateFileW` itself. Either keep each Win32 function host-side, with one Rust
  implementation behind it and its `Nt*` export, or move the thin wrappers into `guest/` over the `Nt*` stubs, as
  Windows does, so each kernel object has one entry point. The `Nt*` exports are needed either way.
- **RPC between processes:** real ALPC under `rpcrt4`, or a host channel of atlas's own with `rpcrt4` and `combase`
  written to it.
- **Which Windows atlas is:** it reports Windows 10 22H2 (10.0.19045); Windows 11 would change manifests, API sets and
  what programs check for.
- **Graphics APIs.** Direct3D, DXGI and OpenGL make thousands of calls a frame, and as `host` exports each would be
  an exit. Real drivers batch calls in the program and submit once a frame. atlas probably needs the same: record
  commands into a buffer in the program, and translate them to Metal or Vulkan host-side at `Present`. That's a
  second, small piece of code in the program. (Host exports, guest code, or a GPU protocol: not decided.)
Settled while building:

- **ucrtbase** is atlas's own, as it's an OS DLL since Windows 10, and atlas's DLLs win; all of it runs in the
  program, reaching kernel32's host exports by number for files.

- **Where the registry is saved**: one file in the machine's own layer, `C:\Windows\System32\config\REGISTRY`, in
  atlas's own format. It goes with the machine and freezes into an image with it.
- **The toolchain for `atlas/guest`**: rustup's prebuilt `core` for the Windows targets links but leaves `memcpy`,
  `memset`, `memcmp` and `strlen` undefined, and ntdll has no C runtime beneath it to supply them, so it's built with
  `build-std` and `compiler-builtins-mem`, always (`RUSTC_BOOTSTRAP`; rustup needs `rust-src`, Homebrew's Rust
  has the source). The same build runs on macOS, Linux and Windows.
