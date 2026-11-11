# winkvm.sys

Carbon's own VT-x driver: the hypervisor behind `vm/winkvm` on Windows x86-64 hosts. It's here, not WHP, because WHP
needs Hyper-V, which fails or conflicts under nested virtualization in the cloud. Intel VT-x with EPT only.

It's the smallest hypervisor `exec` needs, shaped like KVM: one guest RAM region in EPT, vCPUs, registers in and
out, and RUN until the guest halts. The guest's page tables, the shim, and what a stop means are `exec`'s, shared with
KVM. `winkvm.h` is the whole interface; `exec/windows/src/vm/sys.rs` mirrors it, with the same struct sizes asserted on
both sides.

```
winkvm.h               the interface: calls, structures, exit reasons
src/driver.c           the device, its calls, the VM table, cleanup when a handle closes
src/vmx.c              VMX on/off on every processor, the VMCS, the run loop, exits
src/ept.c              guest RAM: locking it, and its EPT
src/vmx_enter.asm      entering and leaving the guest
src/winkvm_internal.h  VMs, vCPUs, VMX's numbers
tests/smoke.c          the driver on its own, with tiny guests
winkvm.vcxproj         the WDK project
build.ps1, install.ps1, uninstall.ps1
```

## How it runs a guest

- **VMX** is switched on on every processor when the driver loads (an IPI to each), and off when it unloads. If any
  processor can't (no VT-x, the firmware locked it off, or another hypervisor holds it) the device still loads and
  `CAPS` says `Vmx = 0`.
- **RUN** happens on the calling thread, held to its current processor for the call, since a VMCS is active on one
  processor at a time. It loads the VMCS, enters, and handles what it can without going back to user mode: host
  interrupts (the reason it exits on them at all) and CPUID. Between entries it drops to `PASSIVE_LEVEL`, so the
  host's interrupts, DPCs and scheduler carry on and a guest spinning forever can't stall the processor. On any other
  exit it reads the guest's state out and clears the VMCS, so the next RUN can be on any processor.
- **CANCEL** sets a flag and queues a high-importance DPC at the processor the vCPU is running on; that interrupt
  makes the guest exit, and RUN returns `WINKVM_EXIT_CANCELED`.
- **Guest RAM** is the caller's buffer, locked for as long as the VM lives, in 256 MiB MDLs. The EPT maps exactly
  it, write-back, and nothing else.
- **Extended state**: the guest's x87/SSE/AVX/AVX-512 state lives in an XSAVE area per vCPU and is swapped in and
  out around every entry, inside `vmx_enter.asm`, which keeps the Windows x64 ABI whole for its C caller. The calling
  thread's own state is saved around the whole call with `KeSaveExtendedProcessorState`.
- **What VM exits don't restore** is put back by hand: GDTR/IDTR limits, DR7, `IA32_DEBUGCTL`, ds and es. The
  syscall MSRs (`STAR`, `LSTAR`, `CSTAR`, `SFMASK`, `KERNEL_GS_BASE`) switch through VMX's MSR load/store lists.

## What the guest can't do

The guest's ring 0 is whatever the caller wrote, so nothing it does reaches the host:

| The guest tries | What happens |
| --- | --- |
| Memory outside guest RAM | EPT violation: RUN returns `OTHER`, reason 48 |
| Any MSR (`RDMSR`/`WRMSR`) | No MSR bitmap, so every access exits: `OTHER`, 31/32 |
| Port I/O | Unconditional I/O exiting: `OTHER`, 30 |
| Debug registers | MOV-DR exiting: `OTHER`, 29 |
| `MWAIT`, `MONITOR`, `RDPMC`, `WBINVD`, `XSETBV`, `INVD`, VMX instructions | They exit: `OTHER` |
| `CPUID` | Answered from the host's, with VMX and "hypervisor present" hidden |
| Bad control registers or segments | VM entry's own checks fail it: `FAILED_ENTRY` |

`MXCSR` from `SET_FP` is masked to what the CPU allows, so a bad value can't make `XRSTOR` fault in the kernel. Inputs
are checked for size, alignment and range; a VM belongs to the handle that made it, and closing that handle destroys
it. The device is open to SYSTEM and Administrators only, so `carbon` runs elevated on Windows.

## Building and testing

You need Visual Studio 2022 (the Build Tools are enough) with the C++ tools, the Spectre-mitigated libraries, the
Windows 11 SDK and the WDK, on Windows 10 2004 or later (it uses `ExAllocatePool2`). With the Build Tools, the WDK's
build integration is the installer's `Component.Microsoft.Windows.DriverKit.BuildTools`, after the WDK itself.

```powershell
bcdedit /set testsigning on        # once, then reboot: the build is test-signed
.\build.ps1                        # build\Release\winkvm.sys and build\Release\smoke.exe
.\install.ps1                      # as Administrator
.\build\Release\smoke.exe          # as Administrator
.\uninstall.ps1
```

`smoke.exe` runs tiny ring-0 guests in long mode straight against the device: halting with a register set, CPUID,
a spinning guest cancelled from another thread, port I/O, `WRMSR`, and memory outside guest RAM. Its exit code is
how many checks failed. On GCP, use an Intel N2/C2/C3 VM with nested virtualization on, and Secure Boot off.

It passes all 17 checks on Windows Server 2022, in a GCP n2-standard-4 VM with nested virtualization; and through it,
exec's conformance suite passes on `vm/winkvm` (18 of 18), as do lambda's and atlas's test programs.

The device must be opened with `FILE_FLAG_OVERLAPPED`: on a synchronous handle the I/O manager runs one call at a
time, so a `CANCEL` from another thread would wait behind the `RUN` it's meant to stop.

## Not yet

- **NMIs** during a guest go to the guest, which reports them as an undefined instruction. Exiting on them and handing
  them back to the host comes later.
- **VPID**: off, so every entry and exit flushes the TLB. Turning it on needs an INVVPID when a vCPU changes processor.
- **Lazy guest RAM**: all of it is locked up front. Locking pages as EPT violations ask for them comes later.
- **Kernel CET** (Windows 11's hardware-enforced kernel stack protection) isn't handled: VM exits would need its
  state loaded.
- **AMD**: SVM is a different interface, not covered.
- **Signing**: outside test mode a kernel driver needs an EV certificate and Microsoft's attestation signing, which
  decides how `winkvm` ships.
