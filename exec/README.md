# exec

`exec` runs a program's machine code and stops it whenever the program needs its kernel: a system call, a fault, a
bad instruction, or a time slice running out. It owns the program's memory (address spaces, pages, copy-on-write)
and its CPUs. It knows nothing of any kernel, file system or ABI.

A kernel sees one type, `Exec`, and one way of running: run a CPU until it stops, take a turn to deal with the stop,
run again. Behind that, each host has exactly two backends, picked by **isolation**. All six run, and pass the same
conformance suite:

| Host | `vm`: a hardware VM | `sandbox`: a process on the host |
| --- | --- | --- |
| macOS, Apple silicon | `hvf`, Hypervisor.framework: ARM64 natively, x86-64 translated | a stub task running rewritten ARM64 code |
| Linux x86-64 | `kvm`, `/dev/kvm` | a stub process under `ptrace`, Syscall User Dispatch, seccomp |
| Windows x86-64 | `winkvm`, Carbon's own VT-x driver | a stub process under the debug API, restricted token and job |

```sh
carbon run --isolation vm      debian:13 -- ./untrusted.sh   # hardware VM or fail
carbon run --isolation sandbox debian:13 -- make             # force the process sandbox
carbon run                     debian:13 -- make             # auto: best available (the default)
```

Program code runs at native speed on both. They differ in the strength of the wall around it, and in what a system
call costs.

## Where it stands

Every backend has been built and run on its own host: macOS on Apple silicon; Linux x86-64 and Windows Server 2022
in GCP VMs with nested virtualization (n2, Intel). On each, the conformance suite passes, and so do the kernels'
test programs (lambda's six: hello, signals, threads, code, and the x86-only `sse` and `x87`; atlas's eight), except
where a backend says it can't run them yet:

| Backend | Conformance | lambda (Linux programs) | atlas (Windows programs) |
| --- | --- | --- | --- |
| `vm/hvf` | 48 of 48 (ARM64, x86-64, 32-bit x86) | all, both ISAs | all, both ISAs |
| `sandbox/macos` | 12 of 12 (ARM64) | not run there yet | not run there yet |
| `vm/kvm` | 25 of 25 (x86-64) | all | all |
| `sandbox/linux` | 24 of 24 | all but fork and threads (one CPU) | all but threads (one CPU) |
| `vm/winkvm` | 25 of 25 | all | all |
| `sandbox/windows` | 9 of 9 (through its gate) | no: needs x86 → x86 rewriting | all that need one CPU |

What the "not yet"s are, in the order they matter, is the to-do list below. `winkvm.sys` passes its own 17 checks
(`windows/driver/README.md`), test-signed.

Which programs each backend runs:

| Backend | ARM64 programs | x86-64 programs | 32-bit x86 programs |
| --- | --- | --- | --- |
| `vm/hvf` | **native** | **translated** | **translated** (exec's own cases; atlas's WoW64 to come) |
| `sandbox/macos` | **rewritten** (one CPU) | translated: not started | not started |
| `vm/kvm` | translated: not started | **native** | native: needs a 32-bit code segment in the shim |
| `sandbox/linux` | translated: not started | **native** (one CPU) | not started |
| `vm/winkvm` | translated: not started | **native** | native: needs a 32-bit code segment in the shim |
| `sandbox/windows` | translated: not started | **native** (one CPU), for atlas; lambda needs x86 → x86 rewriting | not started |

A host with nothing for one isolation still has two backends: `NoVm` or `NoSandbox`, which never open and say why.
Linux ARM64 hosts come later, with kvm for ARM64 and a sandbox that rewrites code as macOS's does.

## The rules exec keeps

1. **Every exit goes through core.** `Cpu::run` returns an opaque `Exit` that only `Turn::stop` can read, so a
   translator's or shim's own traps never reach a kernel by mistake.
2. **Only a CPU's own thread can take its turn.** `Cpu` is `!Send`, and `enter` takes `&mut Cpu`. hvf needs this, and
   the compiler checks it.
3. **A backend is small on purpose.** It answers its contract and nothing more; anything that isn't about one host is
   written once, in core.
4. **The kernel only ever sees the program's ISA.** Translation is core's business: a kernel names an `Isa` in
   `new_space`, and gets `Regs::X86` or `Regs::Arm64` back.
5. **One concrete type.** `Exec` is `exec_core::Core<host::Vm, host::Sandbox>`: a host has exactly two backends, so
   `Exec` dispatches between them with `match`, and no kernel is ever generic over an engine.
6. **Constants live with what they describe.** Facts about a CPU go in `exec_core::isa`, tunable policy in `Tuning`,
   a host API's numbers in that host's `sys.rs`. There is no `consts.rs`.
7. **One conformance suite.** Every backend runs the same tiny programs and must give the same results. A backend is
   done when it passes.
8. **exec depends on nothing of Carbon's.**

## Using it

```rust
use exec::{Exec, Isa, Isolation, Options, Prot, Stop};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    exec::run_if_stub(); // first thing: on macOS the sandbox's stub is this same binary

    let (mut exec, mut cpu) = Exec::open(Options { memory: 512 << 20, isolation: Isolation::Auto, ..Options::default() })?;
    println!("running on {}", exec.backend()); // vm/hvf, sandbox/linux, …

    // A turn: the kernel's time with this CPU. Ending it settles everything (TLBs, the sandbox's stub, memory).
    {
        let mut turn = exec.enter(&mut cpu);
        let space = turn.new_space(Isa::X86_64)?; // translated on Apple silicon
        turn.use_space(space);
        let at = turn.space().layout().top - (64 << 20); // somewhere the backend lets programs live
        turn.space().map(at..at + 0x1000, Prot::RX).with(&program_code())?;
        turn.space().map(at + 0x10_0000..at + 0x20_0000, Prot::RW).zeroed()?;
        turn.thread().start(at, at + 0x20_0000);
    }

    loop {
        let exit = cpu.run(); // without the kernel's lock: programs on other CPUs run in parallel
        let mut turn = exec.enter(&mut cpu);
        match turn.stop(exit) {
            None | Some(Stop::Preempted) => continue,                     // exec dealt with it, or a time slice
            Some(Stop::Syscall { nr, args }) => turn.thread().set_return(answer(nr, args)),
            Some(Stop::Fault { addr, access, .. }) => handle_fault(&mut turn, addr, access)?,
            Some(other) => break Err(format!("the program stopped: {other:?}").into()),
        }
    }
}
```

The whole surface, all in `exec`:

| Type | What it is |
| --- | --- |
| `Exec` | A machine: its memory and CPUs, on this host's `vm` or `sandbox` backend. `open`, `backend`, `caps`, `add_cpu`, `enter`, `memory` |
| `Options` | `memory` (a limit; only what's touched is backed), `isolation`, `tso` (x86 memory ordering on Apple silicon), `tuning` |
| `Cpu` | One CPU, owned by its thread (`!Send`). `run` returns an opaque `Exit`; `handle` gives a `CpuHandle` any thread can `preempt` |
| `Turn` | A CPU's time in the kernel. `stop(exit)` says what the exit means (`None`: exec handled it). Spaces (`new_space`, `fork_space`, `use_space`, `destroy_space`), `preempt`, `time_slices`, `flush_tlb`, `tidy` |
| `Space` | The current address space. `map(range, prot)` then `.zeroed()`, `.with(bytes)`, `.frame(f)` or `.copy_on_write(f)`; `unmap`, `protect`, `move_pages`, `make_writable`, `page`, `read`, `write`, `load_u32`, `update_u32`, `layout`, `thread_block` |
| `Thread` | This CPU's registers, as the program's own CPU has them: `start`, `set_return`, `regs`/`set_regs` (`Regs::X86` or `Regs::Arm64`), `fp`/`set_fp`, `save`/`restore`, `program_pc` |
| `Memory` | The machine's memory as a whole: frames for the kernel, `pressure`, `stats`, `census` |
| `Stop` | `Syscall`, `Fault`, `Undefined`, `Alignment`, `Breakpoint`, `FastFail`, `DivideError`, `Preempted`, `Internal` |
| `run_once` | For tests: code in a fresh space, run to its first real stop |
| `run_if_stub` | Call first in `main`: on macOS, the sandbox's stub is this binary run again |

Rules the types enforce, and a few they can't:

- **Only a CPU's own thread can have its turn** (`enter` takes `&mut Cpu`, which can't leave its thread). hvf needs
  this; the compiler checks it. It also means a second CPU is added from the thread that will run it (hvf allows one
  vCPU per thread).
- **Every exit goes through `Turn::stop`.** `Exit` is opaque, so a translator's or shim's own traps never reach a
  kernel by mistake, and a turn can't end without settling.
- **A shared or not-yet-touched page is unmapped to the kernel's copies**: `write` refuses it. The kernel calls
  `make_writable` (as a fault would) and tries again; `kernels_core::user::with_faults` does this.
- **A backend may have a gate** (`Layout::gate`): its programs call that address to reach their kernel, because the
  host can't trap their system-call instruction. Only the Windows sandbox has one.
- **Drop CPUs before their `Exec`**: hvf can't destroy a VM that still has a vCPU.
- **A dropped CPU's slot is used again** on the vm backends, so a machine can make CPUs for as long as it runs, as many
  at once as `caps().max_cpus`. Core keeps each slot's shim pages; hvf makes a new vCPU, and kvm and winkvm (which
  never take a vCPU back) keep the dropped one for the next CPU, on whatever thread that is, with its FPU reset. A
  sandbox's slot stays with the host thread that drives its stub, so a sandbox makes as many CPUs as it has, once.
  The conformance suite checks it on hvf, kvm and winkvm: `caps().max_cpus + 16` CPUs, each made and dropped in turn.

## Inside

```
exec/
  src/         exec            the facade: picks this host's two backends, re-exports the API
  core/        exec-core       everything written once
    isa/                       registers per CPU type, where each trap's arguments are, flag conversion
    shim/                      what a vm backend puts in its guest: x86 (kvm, winkvm) and ARM64 (hvf)
    sandbox/                   the host-neutral half of a sandbox: mirroring, the gate, faults, layout
    exec.rs turn.rs cpu.rs     spaces, turns, CPUs, settling, time slices, giving memory back
    translated.rs              x86 on an ARM64 vm backend: code cache, block exits, the x86 view of registers
    contract.rs                the traits a backend implements, and nothing else
    tests/mock.rs              core on a mock sandbox: spaces, fork, copy-on-write, mirroring, with no hypervisor
  mmu/         exec-mmu        4-level page tables (x86 and ARM64 formats), frames, copy-on-write
  translate/   exec-translate  x86-64 → ARM64, ARM64 → ARM64 rewriting, the ARM64 assembler, the code cache's books
  macos/       exec-macos      vm/hvf, sandbox (stub task, Mach exceptions, rewritten code)
  linux/       exec-linux      vm/kvm, sandbox (ptrace stub, Syscall User Dispatch, seccomp)
  windows/     exec-windows    vm/winkvm, sandbox (debug-API stub, section views, restricted token)
    driver/                    winkvm.sys: VMX, EPT, the run loop (C and MASM), and its smoke test
  examples/conformance.rs      the conformance suite: the same programs on both of this host's backends
```

**A backend is small on purpose.** A `vm` backend makes CPUs, moves their registers, sets their root page table, runs
them and preempts them; core and the shim do the rest. A `sandbox` backend starts a stub, mirrors pages into it,
moves registers, resumes it and reports what stopped it; core's sandbox half does the rest. Missing backends are
types (`NoVm`, `NoSandbox`) that never open and say why, so every host always has exactly two.

**vm backends** run the program at EL0 or ring 3 with a few pages of shim above it. Every exception goes into the
shim, which halts (x86) or makes a hypercall (ARM64), and core reads where the program was. Page tables live in guest
memory and the hardware walks them. A TLB flush is a generation count: a CPU that's behind reloads its root on the way
back to its program, and running CPUs are stopped and waited for. The x86 shim is shared by kvm and winkvm.

**sandbox backends** run the program natively in a stub process on the host. Core keeps the same page tables and
mirrors each change into the stub as host mappings, a page at a time, when the turn ends. System calls reach the
kernel however the host allows: Syscall User Dispatch on Linux, rewritten `svc` on macOS, and on Windows, which can't
send a `syscall` anywhere but its own kernel, a gate the program calls.

**Translation**: on an ARM64 vm backend, an x86-64 space is translated block by block into a code cache at the top of
its own address space. Blocks are chained: a direct jump's exit is rewritten into a branch to its target once that's
translated, and indirect jumps and returns look their targets up in a table after the code cache, which core fills
as they miss. What still leaves a block (a system call, a miss, something untranslated) is a `brk` that core follows
before the kernel sees anything. x86's registers live in `x0`–`x15` (fs and gs bases in `x28`, `x27`, xmm in
`v0`–`v15`, x87 in `v16`–`v24`), and
ARM's carry always holds x86's CF inverted, so `sub` and `cmp` need nothing and every condition maps to one ARM
condition. The kernel only ever sees x86.

A translation is forgotten when the code it came from changes: when the kernel unmaps, moves, remaps, re-protects or
writes its page, and when the program writes it. A page the program may both write and run loses its write access
while translations from it are live, so its first write stops in core, which forgets them, gives the access back
and lets the write go ahead. Forgetting a block unlinks the chains into it and clears its lookup entry; a CPU
already inside it finishes it. The code cache is used a half at a time (448 MiB of each, by `Tuning::code_cache`):
when one fills, its chains are unlinked, the lookup table emptied, and translation moves to the other half, which is
only written again once no CPU can still be in it. A CPU waiting in its kernel all that time is moved off the old
code by its x86 address at its next turn (`translated.rs` has the details).

Core emulates `cpuid`; the kernel picks the feature set it reports, and must claim only what exec can run (for
translated x86 today, SSE2 and no AVX). On Apple silicon the hardware's TSO mode gives x86 memory ordering, switched on
per vCPU for translated spaces. **ARM64 → ARM64 rewriting** (the macOS sandbox) copies nearly every instruction as it
is; its exits are `brk`s keyed by their address in a side table, so no register of the program's is used. The
decoder's lengths and the assembler's encodings are checked against LLVM's.

### The backend contract

Two pairs of traits in `core/src/contract.rs`. Each host crate implements both pairs, and nothing else.

```rust
pub trait Vm: Sized + Send + 'static {
    type Shim: Shim;                               // shim::x86 (kvm, winkvm) or shim::arm64 (hvf)
    type Cpu: VmCpu<Shim = Self::Shim>;
    const NAME: &'static str;
    const RAM_BASE: u64;                           // where guest RAM starts in guest physical memory
    fn alloc_ram(bytes: u64) -> Result<GuestRam, String>;
    fn open(ram: &GuestRam, options: &Options) -> Result<Self, String>;
    fn max_cpus(&self) -> usize;
    fn tso(&self) -> bool { false }
    fn sync_code(&self, host: *mut u8, len: usize) {}
    fn new_cpu(&self, index: usize) -> Result<Self::Cpu, String>;   // on the calling thread
}

pub trait VmCpu: 'static {
    type Shim: Shim;
    fn set_system(&mut self, system: &<Self::Shim as Shim>::System) -> Result<(), String>;
    fn regs(&mut self) -> &mut <Self::Shim as Shim>::Regs;
    fn fp(&mut self) -> &mut <Self::Shim as Shim>::Fp;
    fn set_root(&mut self, value: u64);            // cr3 / ttbr0
    fn run(&mut self) -> VmExit;                   // Trap { at } | Preempted | Failed(code)
    fn preempt_handle(&self) -> RawPreempt;
}

pub trait Sandbox: Sized + Send + 'static {
    type Cpu: SandboxCpu;
    const NAME: &'static str;
    fn alloc_ram(bytes: u64) -> Result<GuestRam, String>;
    fn open(ram: &GuestRam, options: &Options) -> Result<Self, String>;
    fn isas(&self) -> &'static [Isa];
    fn reach(&self) -> Reach;                      // Dispatch (Linux) | Breakpoint (Windows) | Rewrite (macOS)
    fn layout(&self, isa: Isa) -> Layout;
    fn max_cpus(&self) -> usize;
    fn mirror(&mut self, page: u64, ram_offset: Option<u64>, prot: Prot) -> Result<(), String>;
    fn thread_block(&mut self, size: u64) -> Option<u64> { None }
    fn new_cpu(&self, index: usize) -> Result<Self::Cpu, String>;
}

pub trait SandboxCpu: 'static {
    fn regs(&mut self) -> &mut Regs;
    fn fp(&mut self) -> &mut FpRegs;
    fn resume(&mut self) -> SandboxExit;           // Gate(trap) | Fault(HostFault) | Preempted | Failed(code)
    fn preempt_handle(&self) -> RawPreempt;
}
```

The `Shim` trait is what a `vm` backend puts in its guest: it installs itself into the page tables, adds CPUs, and
turns a trap into a `Stop`. There are two, `shim::x86` (GDT, IDT, TSS, halt stubs) and `shim::arm64` (EL1 vectors
that make a hypercall), both in `exec-core`.

### Each backend

- **`vm/hvf`**: one VM per process, so `machine` runs each hvf machine in a process of its own. Each vCPU belongs to
  the thread that made it. The ARM64 shim's vectors make a hypercall for every exception. TSO is switched on per vCPU
  for translated x86. Preempted with `hv_vcpus_exit`.
- **`sandbox/macos`**: the stub is Carbon's own binary run again (`CARBON_EXEC_STUB`), which signals it's ready on a
  pipe. Carbon takes its task with `task_for_pid`, makes a thread in it for the program, catches that thread's Mach
  exceptions, and copies memory in 16 KiB windows. XNU maps nothing below 4 GiB in a 64-bit process, so programs live
  above it. A task port the stub sends itself can't be used: it's guarded, and using it kills the receiver. That's
  why Carbon's binary needs `get-task-allow`.
- **`vm/kvm`**: needs `KVM_CAP_SYNC_REGS` and `KVM_CAP_IMMEDIATE_EXIT`. There's no in-kernel interrupt controller: a
  halt in a shim stub exits to the host. Preempted by a real-time signal with `immediate_exit`. Guest RAM is a memfd,
  shared with the sandbox's RAM code.
- **`sandbox/linux`**: a stub driven by `ptrace`. Syscall User Dispatch sends every `syscall` to the gate, with the
  selector byte read-only to the program. seccomp narrows what the stub itself may do, and denies it networking.
- **`vm/winkvm` and `winkvm.sys`**: Carbon's own VT-x driver, because WHP needs Hyper-V, which fails or conflicts
  under nested virtualization in the cloud. VMX is turned on on every CPU by IPI. `RUN` is pinned to one CPU and does a
  VMCLEAR at the end. Guest RAM is mapped through EPT. XSAVE state is swapped inside the assembly, MSRs through the
  VMCS's load and store lists, and `CANCEL` is a targeted DPC. The guest is the same x86 guest as kvm's.
- **`sandbox/windows`**: a stub under the debug API, from one dedicated thread. Guest RAM is a section, mapped into
  the stub as views at a 64 KiB stride with placeholders. The stub runs with a low-integrity restricted token inside a
  job. atlas reaches the gate through a breakpoint. The stub's TEB is Windows', so `thread_block` places it, and it's
  copied in and out.

### Decisions made while building it

- **The shims live in `exec-core`** (`core/src/shim/`), not a crate of their own. Core drives them and they drive
  core's page tables, so a separate crate would only make a cycle. `isa`, `Tuning` and the types live there too, and
  `exec` re-exports them.
- **`Exec::open` returns the first CPU with the machine**: `let (mut exec, mut cpu) = Exec::open(options)?`. A `Cpu`
  belongs to its thread, so it can't wait inside `Exec`, which moves between threads.
- **`make_writable(page, prot)`** takes the access to give the copy, since a shared page's entry has lost its write
  bit.
- **No `--backend`**: a host has two backends, so `--isolation` already names one.
- **The macOS sandbox does its own rewriting** for now, inside its backend, rather than through core.

### Why it's shaped this way

| The easy way | What goes wrong | Here |
| --- | --- | --- |
| One big engine trait every backend implements | Memory bookkeeping written once per backend; kvm and winkvm near-identical | A small contract per isolation, and core does the rest |
| A backend keeping its own page tables beside the shared ones | Two sources of truth | One `exec-mmu`, with both formats |
| x86 registers packed into ARM64's struct | Every reader has to know the trick | `Regs` per ISA |
| Translation inside one backend | No other backend can use it | `exec-translate`, driven by core |
| Settling and filtering exits as rules in comments | Easy to forget | `Turn` settles when it ends; `Exit` must go through `Turn::stop` |
| Trusting a backend because its programs finish | A software interpreter can stand in for the hardware, unnoticed | Conformance tests, and a driver with its own tests |
| Kernels generic over the engine | Every kernel type carries the engine | One concrete `Exec` |

## Building and testing

```sh
cargo build --workspace              # each host builds its own backends; the others are cfg'd out
cargo clippy --workspace --all-targets
cargo test -p exec-translate         # encodings and decoding checked against LLVM's
cargo test -p exec-core --test mock  # core on a mock sandbox, on any machine

# The conformance suite, on this host's vm and sandbox backends:
cargo build --release --example conformance
codesign --force --sign - --entitlements carbon.entitlements target/release/examples/conformance   # macOS only
target/release/examples/conformance            # or: conformance vm, conformance sandbox
```

Per host:

- **macOS**: binaries that open a backend are signed with `carbon.entitlements`: `com.apple.security.hypervisor`
  (hvf), `com.apple.security.cs.debugger` (to take the sandbox stub's task) and `get-task-allow` (to let it be
  taken). The stub is the same binary, so the last also makes Carbon debuggable by the same user.
- **Linux**: kvm needs `/dev/kvm` and the user in the `kvm` group. The sandbox needs nothing.
- **Windows**: MSVC (Visual Studio 2022 Build Tools); `winkvm.sys` built with the WDK, test signing on
  (`bcdedit /set testsigning on`, then reboot), installed and started as Administrator (`windows/driver/README.md`).
  Carbon runs elevated, to open `\\.\WinKvm`. The sandbox needs nothing.

**The conformance suite** runs 27 tiny programs, assembled with LLVM from the assembly beside each, in every ISA each
backend runs, and checks the same stops, registers and memory on all of them:

- exit, a system call's six arguments, and its answer;
- a loop, and a store the kernel reads back;
- fork with copy-on-write;
- writing a read-only page, reading an unmapped one, and a fault the kernel answers before carrying on;
- a breakpoint and an undefined instruction at their own addresses, and divide by zero;
- preemption from another thread, and a second CPU taking a program's registers;
- the translator's checks (call and `cpuid`, `adc`/`sbb` at every size, the bit tests, `shld`/`shrd`, a 128-bit
  `div` and one that overflows);
- code that changes after it ran: rewritten by the program on a page it may write, written over by the kernel, and
  mapped anew where it was; and more code than a 64 KiB code cache holds, run through eight times;
- on a backend with a gate, exit and an answer through it.

A case skips, saying why, where a backend can't run it: an ISA it doesn't run, a second CPU it doesn't have, or a
system-call instruction it can't trap.

---

## To do

In order: what stops real programs running somewhere, then what makes them fast, then what widens where exec runs.
The order across all of Carbon is in the [top-level README](../README.md#the-order-of-work); this is exec's part of
it, in more detail. When an item is done it's ticked here, with what it took.

### 1. The sandboxes, finished

- [ ] **More than one CPU in each sandbox**: one stub thread per CPU, sharing the stub's address space, each driven
      by the host thread that owns its CPU (on Linux and Windows, the thread that traces or debugs it). lambda's fork
      and atlas's `CreateThread` need it; it's the last thing between the sandboxes and the vm backends' results.
- [ ] **Linux**: map runs of pages per call, not one page per `ptrace` round trip.
- [ ] **Windows**: lambda's `syscall` (x86 → x86 rewriting into gate calls); today it runs atlas only. The
      conformance suite's system-call cases in gate form (fork, a fault answered, the translator's checks), so the
      sandbox runs all of them.
- [ ] **macOS**: run the kernels' test programs on it. Share memory instead of copying (16 KiB windows whose frames
      line up as views, copying only the rest, with different protections inside a window: guard pages don't fault
      today); `TPIDR_EL0`; pointer-authenticated branches and literal loads into SIMD registers.

### 2. x86-64 programs as they're built

- [x] **SSE and SSE2** in the translator (`emit/sse.rs`): moves, shuffles, packs, integer and float arithmetic,
      conversions with x86's integer-indefinite answers, compares, `ucomis*` flags, `movmsk*`, and MXCSR on FPCR. The
      test programs are built as they come; Debian 13's x86-64 bash runs, with `ls`, `cat`, `sort`, `grep`, `wc`.
      MMX too, the same integer operations on `mm0`–`mm7`, which are x87's `d16`–`d23` as on an x87, for the 32-bit
      crypto code that uses them.
- [x] **x87** (`emit/x87.rs`), at double precision: ST(0)–ST(7) in `d16`–`d23`, moved on a push or pop so TOP stays
      0; loads and stores of every format, exact 80-bit conversions, the arithmetic, compares into C0–C3 and into
      flags, `fcmov`, `fist` with the control word's rounding, `fprem` in exact parts, `fscale`, `fxam`, the control
      word and environment; `sahf`, `lahf`. bash's `printf "%f"`, `seq`, `awk` and `numfmt` run. Not yet: the
      transcendentals (`fsin`, `fpatan`, `fyl2x`, `f2xm1`, …), `fprem1`, `fxtract`, `fsave`/`frstor`, BCD; and
      80-bit precision, which shows only past a double's 17 digits (`printf "%.20f" 0.1`).
- [x] What Debian's x86-64 `apt`, `dpkg`, `sqv`, Python and git asked for: `fxsave`/`fxrstor` (glibc's lazy binding);
      8- and 16-bit rotates, `mul`, `imul` (one and two operands) and `div`/`idiv`; `lock bts`/`btr`/`btc`, and the bit
      tests on memory with a register's bit number, which can reach past the operand; `stc`, `clc`, `cmc`; and SSE2's
      `pmuludq`, `pmulhw`, `pmulhuw`, `pmaddwd`, `psadbw`, and shifts by a register's count. `sse.c` and lambda's
      `x86.c` check them.
- [x] SSE3's horizontal adds and subtracts, `movddup`, `movsldup`, `movshdup`, `lddqu` (Windows' compilers use them).
- [x] **32-bit x86** in the translator, for atlas's WoW64: the decoder's 32-bit mode (no REX, so `40`–`4F` are
      `inc` and `dec`; 32-bit addresses; `[disp32]` absolute; 4-byte `moffs`), and the emitter's (a 32-bit stack for
      `push`, `pop`, `call`, `ret` and indirect branches; string instructions stepping in 32 bits; `pusha`, `popa`;
      `cmpxchg8b` as one `casal`). An `Isa::X86_32` space on hvf is translated, with its eight registers' top halves
      kept clear and ID and AC kept in `x8`. `pushf` and `popf` in both modes (DF only as a block knows it: a `popf`
      that sets it leaves as UNSUPPORTED), `mov` to and from `moffs`, and `ret imm16` past 4 KiB. Eight conformance
      cases check it: calls and stdcall's `ret 8`, the ID bit, `cmpxchg8b`, fs, `int 0x2e`'s arguments, a fault's pc.
      Not yet: 16-bit addressing (`67`), segment registers as operands, far calls, BCD, `bound`, `enter`, `loop`.
- [ ] SSE3's `addsub`, SSSE3 and later, and AVX.
- [ ] The parity flag beyond `ucomis*` and `fcomi`; string instructions beyond `movs` and `stos` (which step down
      with DF set, as `std` and `cld` leave it within a block); exact flags after `mul` and `imul` (CF and OF are; SF, ZF and PF aren't); `rcl`, `rcr`; signed
      128-bit divides (unsigned ones are done); `cmpxchg16b`; `lock adc`/`sbb`; `syscall`'s `r11`.

### 3. Speed

- [x] **Block chaining**: direct jumps linked by rewriting their exits, indirect jumps and returns through an
      in-guest lookup table (16,384 one-word entries after the code cache). With the end-of-turn fix below, Debian's
      x86-64 `seq 100000` went from 78 s to 0.06 s, a 100,000-round bash loop from over 150 s to 1.5 s, and a
      call/return loop runs at about 3 ns a round. `threads.c` does its full rounds on x86-64 now.
- [x] **The end of every turn** looked at every granule of guest memory for some to give back (`Frames::excess`);
      the counts are kept as frames come and go now, so it's O(1) when there's nothing to give. A block exit or a
      system call went from about 39 µs to about 2 µs.
- [x] **Forgetting translations when their code changes**, and a full code cache emptied safely while other CPUs
      run (see Translation, above). Four conformance cases, and lambda's `code.c` on both ISAs: code rewritten 1,000
      times, a direct call into rewritten code, W^X, `munmap` then new code at the same address, another thread
      rewriting a function this one keeps calling, and 4,096 functions rewritten 16 times while another thread runs
      its own. With `CARBON_EXEC_CODE_CACHE=65536` (the runners' knob for `Tuning::code_cache`) that last part moves
      the cache between halves about 70 times; 100 runs in a row pass.
- [ ] **What forgetting doesn't catch yet**: code written through a second mapping of the same memory (a JIT that
      maps its code twice, writable and runnable, as .NET does; `MAP_SHARED` memory another process writes), since
      core guards the page it translated from, not the memory under it. And a block that rewrites one of its own
      later instructions runs the old one: the change shows from the next block.
- [ ] **A real JIT**: a JavaScript engine (or LuaJIT) running in an x86-64 image, the test the to-do list set.
- [ ] ASIDs (ARM64) and VPIDs (x86, and in winkvm): switching spaces flushes the CPU's whole TLB today.
- [ ] hvf moves every register in and out on every exit; move only what's used.
- [ ] One translation runtime in core for the vm backends and the macOS sandbox (today the sandbox's rewriting lives
      in its backend), which a Linux ARM64 sandbox would use too.
- [ ] Share translated x86 code between processes, per image: each process translates its own, which costs startup
      time more than memory.
- [ ] Cache what's translated on disk, so it needn't be done again on each run.
- [ ] Mixed ISAs in one process, as ARM64EC does: a translated x64 program calling an ARM64 build of atlas's ntdll
      through thunks at its forwarders, so `memcpy` and `HeapAlloc` run natively. It needs the translator to call out
      to native code.

### 4. Safety and robustness

- [ ] **Windows sandbox**: a stronger token (restricting SIDs or an AppContainer). A program that executes `syscall`
      or `int 0x2e` itself makes a real NT call in the stub, and only its low-integrity token stops it. AVX state;
      confirm placeholders allow page-granular views everywhere, or fall back.
- [ ] **winkvm.sys**: NMIs during a guest go to the guest instead of the host; lock guest RAM as EPT violations ask,
      not all up front; kernel CET (Windows 11); signing outside test mode (EV certificate, attestation), which decides
      how it ships.
- [ ] **hvf**: keep the VM until its last CPU is dropped, so dropping an `Exec` first can't leave it behind.
- [ ] **macOS sandbox**: a way to take the stub's task that doesn't make Carbon itself debuggable (`get-task-allow`).
- [ ] `CARBON_EXEC_DEBUG` logs stops; add per-backend detail (instruction bytes, registers).

### 5. More hosts and ISAs

- [ ] **32-bit x86 programs** (installers): 32-bit translation on ARM64, 32-bit code segments in kvm's and winkvm's
      shim, and the Linux sandbox's.
- [ ] **Linux ARM64**: kvm with the ARM64 shim; a sandbox, rewriting as macOS's does unless Syscall User Dispatch is
      there; barriers where the CPU has no TSO.
- [ ] **AMD**: SVM in winkvm (kvm already covers AMD on Linux).
- [ ] **ARM64 → x86-64** translation, for ARM64 programs on x86 hosts.
- [ ] **AVX, AVX2, FMA, BMI and F16C** for games and large programs, once SSE3 and later are in; AES, PCLMUL and SHA
      (decoded, not offered) if crypto is slow. SSE4.2's string compares should not leave the VM each time.

## Open questions

- **The macOS sandbox's page size.** A 16 KiB host page can't carry different protections for four 4 KiB guest
  pages. Use the strictest and catch the rest in software, or check every access in translated code? The kernels
  have to accept a coarser protection than a program asked for, and say when that breaks something, such as a guard
  page inside a 16 KiB page.
- **`get-task-allow` on macOS.** The macOS sandbox needs it on Carbon's binary, which also makes Carbon debuggable by
  the same user. Is there a way to take the stub's task without it?
- **Signing the Windows driver.** Outside test mode, a kernel driver needs an EV certificate and Microsoft's signing
  (attestation). That decides how `winkvm` ships.
- **Syscall User Dispatch on Linux ARM64.** If it's missing, that sandbox rewrites code, as on macOS.
